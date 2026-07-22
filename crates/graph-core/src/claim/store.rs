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

/// Input builder for creating an initial claim version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimInput {
    pub(crate) id: ClaimId,
    pub(crate) statement: ClaimStatement,
    pub(crate) target: ClaimTarget,
    pub(crate) confidence: Option<Confidence>,
    pub(crate) created_by: Option<ActorId>,
    pub(crate) source_refs: Vec<String>,
    pub(crate) evidence_refs: Vec<EvidenceId>,
    pub(crate) workspace_id: Option<WorkspaceId>,
    pub(crate) extraction_run_id: Option<ExtractionRunId>,
    pub(crate) temporal: TemporalMetadata,
}

impl ClaimInput {
    /// Creates a new instance.
    pub fn new(id: ClaimId, statement: ClaimStatement, target: ClaimTarget) -> Self {
        Self {
            id,
            statement,
            target,
            // Confidence.
            confidence: None,
            // Created by.
            created_by: None,
            // Source refs.
            source_refs: Vec::new(),
            // Evidence refs.
            evidence_refs: Vec::new(),
            // Workspace id.
            workspace_id: None,
            // Extraction run id.
            extraction_run_id: None,
            // Temporal.
            temporal: TemporalMetadata::default(),
        }
    }

    /// Sets the confidence.
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets the created by.
    pub fn with_created_by(mut self, created_by: ActorId) -> Self {
        self.created_by = Some(created_by);
        self
    }

    /// Sets the source ref.
    pub fn with_source_ref(mut self, source_ref: impl Into<String>) -> Self {
        self.source_refs.push(source_ref.into());
        self
    }

    /// Sets the evidence ref.
    pub fn with_evidence_ref(mut self, evidence_ref: EvidenceId) -> Self {
        self.evidence_refs.push(evidence_ref);
        self
    }

    /// Sets the workspace id.
    pub fn with_workspace_id(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    /// Sets the extraction run id.
    pub fn with_extraction_run_id(mut self, extraction_run_id: ExtractionRunId) -> Self {
        self.extraction_run_id = Some(extraction_run_id);
        self
    }

    /// Sets the temporal.
    pub fn with_temporal(mut self, temporal: TemporalMetadata) -> Self {
        self.temporal = temporal;
        self
    }
}

/// First-class claim record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub(crate) id: ClaimId,
    pub(crate) version_id: ClaimVersionId,
    pub(crate) version: u64,
    pub(crate) status: ClaimStatus,
    pub(crate) statement: ClaimStatement,
    pub(crate) target: ClaimTarget,
    pub(crate) confidence: Option<Confidence>,
    pub(crate) created_by: Option<ActorId>,
    pub(crate) source_refs: Vec<String>,
    pub(crate) evidence_refs: Vec<EvidenceId>,
    pub(crate) workspace_id: Option<WorkspaceId>,
    pub(crate) extraction_run_id: Option<ExtractionRunId>,
    pub(crate) temporal: TemporalMetadata,
}

impl Claim {
    /// Id.
    pub fn id(&self) -> &ClaimId {
        &self.id
    }

    /// Version id.
    pub fn version_id(&self) -> &ClaimVersionId {
        &self.version_id
    }

    /// Version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Status.
    pub fn status(&self) -> ClaimStatus {
        self.status
    }

    /// Statement.
    pub fn statement(&self) -> &ClaimStatement {
        &self.statement
    }

    /// Target.
    pub fn target(&self) -> &ClaimTarget {
        &self.target
    }

    /// Confidence.
    pub fn confidence(&self) -> Option<Confidence> {
        self.confidence
    }

    /// Created by.
    pub fn created_by(&self) -> Option<&ActorId> {
        self.created_by.as_ref()
    }

    /// Source refs.
    pub fn source_refs(&self) -> &[String] {
        self.source_refs.as_slice()
    }

    /// Evidence refs.
    pub fn evidence_refs(&self) -> &[EvidenceId] {
        self.evidence_refs.as_slice()
    }

    /// Workspace id.
    pub fn workspace_id(&self) -> Option<&WorkspaceId> {
        self.workspace_id.as_ref()
    }

    /// Extraction run id.
    pub fn extraction_run_id(&self) -> Option<&ExtractionRunId> {
        self.extraction_run_id.as_ref()
    }

    /// Temporal.
    pub fn temporal(&self) -> &TemporalMetadata {
        &self.temporal
    }
}

/// In-memory claim store reserved as the first claim lifecycle boundary.
#[derive(Clone, Debug, Default)]
pub struct ClaimStore {
    pub(crate) claims: HashMap<ClaimId, Claim>,
    pub(crate) known_evidence: HashSet<EvidenceId>,
    pub(crate) claim_links: Vec<ClaimLink>,
    pub(crate) claim_decisions: HashMap<ClaimId, Vec<ClaimDecision>>,
    pub(crate) stances_by_id: HashMap<String, AgentStance>,
    pub(crate) hypothesis_workspaces: HashMap<HypothesisWorkspaceId, HypothesisWorkspace>,
    pub(crate) hypothesis_claim_membership: HashMap<HypothesisWorkspaceId, Vec<ClaimId>>,
    pub(crate) hypothesis_stance_membership: HashMap<HypothesisWorkspaceId, Vec<String>>,
    pub(crate) known_trust_subjects: HashSet<String>,
    pub(crate) trust_inputs_by_id: HashMap<String, TrustInput>,
    pub(crate) trust_input_ids_by_subject: HashMap<String, Vec<String>>,
    pub(crate) trust_input_ids_by_claim: HashMap<ClaimId, Vec<String>>,
    pub(crate) resolution_policies: HashMap<String, RegisteredEpistemicResolutionPolicy>,
    pub(crate) claim_explanations: HashMap<ClaimId, Vec<EpistemicExplanation>>,
    pub(crate) link_explanations: HashMap<String, EpistemicExplanation>,
    pub(crate) resolution_explanations: HashMap<String, EpistemicExplanation>,
}

impl ClaimStore {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self {
            // Claims.
            claims: HashMap::new(),
            // Known evidence.
            known_evidence: HashSet::new(),
            // Claim links.
            claim_links: Vec::new(),
            // Claim decisions.
            claim_decisions: HashMap::new(),
            // Stances by id.
            stances_by_id: HashMap::new(),
            // Hypothesis workspaces.
            hypothesis_workspaces: HashMap::new(),
            // Hypothesis claim membership.
            hypothesis_claim_membership: HashMap::new(),
            // Hypothesis stance membership.
            hypothesis_stance_membership: HashMap::new(),
            // Known trust subjects.
            known_trust_subjects: HashSet::new(),
            // Trust inputs by id.
            trust_inputs_by_id: HashMap::new(),
            // Trust input ids by subject.
            trust_input_ids_by_subject: HashMap::new(),
            // Trust input ids by claim.
            trust_input_ids_by_claim: HashMap::new(),
            // Resolution policies.
            resolution_policies: HashMap::new(),
            // Claim explanations.
            claim_explanations: HashMap::new(),
            // Link explanations.
            link_explanations: HashMap::new(),
            resolution_explanations: HashMap::new(),
        }
    }

    /// Create a bounded hypothesis workspace for competing interpretations.
    pub fn create_hypothesis_workspace(
        &mut self,
        input: HypothesisWorkspaceInput,
    ) -> Result<HypothesisWorkspaceId, GraphError> {
        if input.title.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "hypothesis workspace title must not be empty".to_owned(),
            ));
        }

        if input.description.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "hypothesis workspace description must not be empty".to_owned(),
            ));
        }

        if self.hypothesis_workspaces.contains_key(&input.id) {
            return Err(GraphError::InvalidVersionState(format!(
                "hypothesis workspace already exists: {}",
                input.id.as_str()
            )));
        }

        let workspace_id = input.id.clone();
        let workspace = HypothesisWorkspace {
            id: input.id,
            title: input.title,
            description: input.description,
            status: HypothesisWorkspaceStatus::Active,
            owner_actor: input.owner_actor,
            created_at: input.created_at,
            parent_context_ref: input.parent_context_ref,
        };

        self.hypothesis_workspaces
            .insert(workspace_id.clone(), workspace);

        Ok(workspace_id)
    }

    /// Read a hypothesis workspace by ID.
    pub fn hypothesis_workspace_by_id(
        &self,
        workspace_id: &HypothesisWorkspaceId,
    ) -> Result<&HypothesisWorkspace, GraphError> {
        self.hypothesis_workspaces
            .get(workspace_id)
            .ok_or_else(|| GraphError::HypothesisWorkspaceNotFound(workspace_id.clone()))
    }

    /// Attach a claim to a hypothesis workspace.
    pub fn attach_claim_to_hypothesis_workspace(
        &mut self,
        workspace_id: HypothesisWorkspaceId,
        claim_id: ClaimId,
    ) -> Result<(), GraphError> {
        self.ensure_hypothesis_workspace_exists(&workspace_id)?;
        self.ensure_claim_exists(&claim_id)?;

        let members = self
            .hypothesis_claim_membership
            .entry(workspace_id)
            .or_default();

        if !members.iter().any(|existing| existing == &claim_id) {
            members.push(claim_id);
        }

        Ok(())
    }

    /// List claims attached to a hypothesis workspace.
    pub fn list_claims_in_hypothesis_workspace(
        &self,
        workspace_id: &HypothesisWorkspaceId,
    ) -> Result<Vec<ClaimId>, GraphError> {
        self.ensure_hypothesis_workspace_exists(workspace_id)?;

        Ok(self
            .hypothesis_claim_membership
            .get(workspace_id)
            .cloned()
            .unwrap_or_default())
    }

    /// Attach a stance record to a hypothesis workspace.
    pub fn attach_stance_to_hypothesis_workspace(
        &mut self,
        workspace_id: HypothesisWorkspaceId,
        stance_id: String,
    ) -> Result<(), GraphError> {
        self.ensure_hypothesis_workspace_exists(&workspace_id)?;
        let _ = self.stance_by_id(stance_id.as_str())?;

        let members = self
            .hypothesis_stance_membership
            .entry(workspace_id)
            .or_default();

        if !members
            .iter()
            .any(|existing| existing == stance_id.as_str())
        {
            members.push(stance_id);
        }

        Ok(())
    }

    /// List stance IDs attached to a hypothesis workspace.
    pub fn list_stances_in_hypothesis_workspace(
        &self,
        workspace_id: &HypothesisWorkspaceId,
    ) -> Result<Vec<String>, GraphError> {
        self.ensure_hypothesis_workspace_exists(workspace_id)?;

        Ok(self
            .hypothesis_stance_membership
            .get(workspace_id)
            .cloned()
            .unwrap_or_default())
    }

    /// Mark a hypothesis workspace status.
    pub fn set_hypothesis_workspace_status(
        &mut self,
        workspace_id: HypothesisWorkspaceId,
        status: HypothesisWorkspaceStatus,
    ) -> Result<HypothesisWorkspace, GraphError> {
        let workspace = self
            .hypothesis_workspaces
            .get_mut(&workspace_id)
            .ok_or_else(|| GraphError::HypothesisWorkspaceNotFound(workspace_id.clone()))?;

        workspace.status = status;
        Ok(workspace.clone())
    }

    /// Register a subject reference that can receive trust inputs.
    pub fn register_trust_subject(&mut self, subject_ref: String) {
        if subject_ref.trim().is_empty() {
            return;
        }

        self.known_trust_subjects.insert(subject_ref);
    }

    /// Create a trust input record.
    pub fn create_trust_input(&mut self, input: TrustInputInput) -> Result<String, GraphError> {
        validate_trust_input_value(input.value)?;

        if input.subject_ref.trim().is_empty() {
            return Err(GraphError::TrustSubjectNotFound(input.subject_ref));
        }

        if !self
            .known_trust_subjects
            .contains(input.subject_ref.as_str())
        {
            return Err(GraphError::TrustSubjectNotFound(input.subject_ref));
        }

        for claim_id in &input.claim_refs {
            self.ensure_claim_exists(claim_id)?;
        }

        let trust_input_id = format!("trust-input--{}", self.trust_inputs_by_id.len() + 1);
        let trust_input = TrustInput {
            trust_input_id: trust_input_id.clone(),
            kind: input.kind,
            subject_ref: input.subject_ref.clone(),
            value: input.value,
            provenance_ref: input.provenance_ref,
            reason_ref: input.reason_ref,
            temporal: input.temporal,
            claim_refs: input.claim_refs.clone(),
        };

        self.trust_inputs_by_id
            .insert(trust_input_id.clone(), trust_input);

        let subject_refs = self
            .trust_input_ids_by_subject
            .entry(input.subject_ref)
            .or_default();
        subject_refs.push(trust_input_id.clone());

        let mut seen_claims = HashSet::new();
        for claim_id in input.claim_refs {
            if seen_claims.insert(claim_id.clone()) {
                let claim_refs = self.trust_input_ids_by_claim.entry(claim_id).or_default();
                claim_refs.push(trust_input_id.clone());
            }
        }

        Ok(trust_input_id)
    }

    /// Read trust inputs by subject reference.
    pub fn trust_inputs_by_subject(
        &self,
        subject_ref: &str,
    ) -> Result<Vec<TrustInput>, GraphError> {
        if !self.known_trust_subjects.contains(subject_ref) {
            return Err(GraphError::TrustSubjectNotFound(subject_ref.to_owned()));
        }

        let trust_input_ids = self
            .trust_input_ids_by_subject
            .get(subject_ref)
            .cloned()
            .unwrap_or_default();

        Ok(trust_input_ids
            .into_iter()
            .filter_map(|id| self.trust_inputs_by_id.get(id.as_str()).cloned())
            .collect())
    }

    /// Read trust inputs associated with a claim.
    pub fn trust_inputs_for_claim(
        &self,
        claim_id: &ClaimId,
    ) -> Result<Vec<TrustInput>, GraphError> {
        self.ensure_claim_exists(claim_id)?;

        let trust_input_ids = self
            .trust_input_ids_by_claim
            .get(claim_id)
            .cloned()
            .unwrap_or_default();

        Ok(trust_input_ids
            .into_iter()
            .filter_map(|id| self.trust_inputs_by_id.get(id.as_str()).cloned())
            .collect())
    }

    /// Register a named deterministic policy entry.
    pub fn register_resolution_policy(
        &mut self,
        registration: EpistemicResolutionPolicyRegistration,
    ) -> Result<(), GraphError> {
        if registration.name.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "resolution policy name must not be empty".to_owned(),
            ));
        }

        let policy = RegisteredEpistemicResolutionPolicy {
            name: registration.name.clone(),
            kind: registration.kind,
            metadata: registration.metadata,
        };

        self.resolution_policies.insert(registration.name, policy);
        Ok(())
    }

    /// Read a named registered policy entry.
    pub fn resolution_policy_by_name(
        &self,
        policy_name: &str,
    ) -> Result<&RegisteredEpistemicResolutionPolicy, GraphError> {
        self.resolution_policies
            .get(policy_name)
            .ok_or_else(|| GraphError::ResolutionPolicyNotFound(policy_name.to_owned()))
    }

    /// Resolve one claim with the selected deterministic policy.
    pub fn resolve_claim_with_policy(
        &self,
        claim_id: &ClaimId,
        policy_name: &str,
    ) -> Result<EpistemicResolution, GraphError> {
        let policy_entry = self.resolution_policy_by_name(policy_name)?;
        let claim = self.claim_by_id(claim_id)?.clone();
        let stances = self.stances_by_claim(claim_id)?;

        let links: Vec<ClaimLink> = self
            .claim_links
            .iter()
            .filter(|link| {
                if link.target_claim_id() == claim_id {
                    return true;
                }

                match link.source() {
                    ClaimLinkSource::Claim(source_claim_id) => source_claim_id == claim_id,
                    ClaimLinkSource::Evidence(_) => false,
                }
            })
            .cloned()
            .collect();

        let mut context =
            EpistemicResolutionContext::new(claim.clone()).with_temporal(claim.temporal().clone());
        for link in links {
            context = context.with_link(link);
        }
        for stance in stances {
            context = context.with_stance(stance);
        }
        for (key, value) in &policy_entry.metadata {
            context = context.with_policy_metadata(key.clone(), value.clone());
        }

        let result = match policy_entry.kind {
            EpistemicResolutionPolicyKind::ConservativeDeterministic => {
                ConservativeDeterministicPolicy.evaluate(&context)
            }
        };

        result.map_err(|error| GraphError::ResolutionPolicyEvaluationFailed(error.to_string()))
    }

    /// Explain claim-level epistemic operations.
    pub fn explain_claim(
        &self,
        claim_id: &ClaimId,
    ) -> Result<Vec<EpistemicExplanation>, GraphError> {
        self.claim_explanations
            .get(claim_id)
            .cloned()
            .ok_or_else(|| GraphError::ClaimExplanationNotFound(claim_id.clone()))
    }

    /// Explain a claim-link operation.
    pub fn explain_claim_link(&self, link: &ClaimLink) -> Result<EpistemicExplanation, GraphError> {
        let link_key = claim_link_explanation_key(link);
        self.link_explanations
            .get(link_key.as_str())
            .cloned()
            .ok_or(GraphError::ClaimLinkExplanationNotFound(link_key))
    }

    /// Record a resolution-output explanation entry.
    pub fn record_resolution_explanation(
        &mut self,
        resolution_ref: String,
        consumed_inputs: Vec<String>,
        actor_ref: Option<String>,
        session_ref: Option<String>,
        workspace_ref: Option<WorkspaceId>,
        reason_ref: Option<String>,
    ) -> Result<(), GraphError> {
        if resolution_ref.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "resolution reference must not be empty".to_owned(),
            ));
        }

        let mut explanation = EpistemicExplanation::new(
            resolution_ref.clone(),
            EpistemicExplanationKind::ResolutionOutput,
        )
        .with_actor_ref(actor_ref)
        .with_session_ref(session_ref)
        .with_workspace_ref(workspace_ref.map(|ws| ws.as_str().to_owned()))
        .with_reason_ref(reason_ref);

        for consumed in consumed_inputs {
            explanation = explanation.with_consumed_input(consumed);
        }

        self.resolution_explanations
            .insert(resolution_ref, explanation);
        Ok(())
    }

    /// Explain a recorded resolution-output entry.
    pub fn explain_resolution_output(
        &self,
        resolution_ref: &str,
    ) -> Result<EpistemicExplanation, GraphError> {
        self.resolution_explanations
            .get(resolution_ref)
            .cloned()
            .ok_or_else(|| GraphError::ResolutionExplanationNotFound(resolution_ref.to_owned()))
    }

    /// Create a new stance record for an agent and claim.
    pub fn create_agent_stance(&mut self, input: AgentStanceInput) -> Result<String, GraphError> {
        self.ensure_claim_exists(&input.claim_id)?;

        if input.agent_ref.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "agent stance agent_ref must not be empty".to_owned(),
            ));
        }

        if let Some(confidence) = input.confidence {
            validate_stance_confidence(confidence)?;
        }

        let stance_id = format!("stance--{}", self.stances_by_id.len() + 1);
        let stance = AgentStance {
            stance_id: stance_id.clone(),
            agent_ref: input.agent_ref,
            claim_id: input.claim_id,
            workspace_id: input.workspace_id,
            stance: input.stance,
            confidence: input.confidence,
            reason_refs: input.reason_refs,
        };

        let explanation = EpistemicExplanation::new(
            format!("claim:{}", stance.claim_id().as_str()),
            EpistemicExplanationKind::StanceUpdate,
        )
        .with_consumed_input(format!("stance:{}", stance.stance_id()))
        .with_workspace_ref(stance.workspace_id().map(|ws| ws.as_str().to_owned()));

        let claim_id = stance.claim_id().clone();

        self.stances_by_id.insert(stance_id.clone(), stance);
        self.record_claim_explanation(&claim_id, explanation);
        Ok(stance_id)
    }

    /// Update an existing stance record by stance ID.
    pub fn update_agent_stance(
        &mut self,
        stance_id: &str,
        patch: AgentStancePatch,
    ) -> Result<AgentStance, GraphError> {
        let stance = self
            .stances_by_id
            .get_mut(stance_id)
            .ok_or_else(|| GraphError::StanceNotFound(stance_id.to_owned()))?;

        if let Some(confidence) = patch.confidence {
            validate_stance_confidence(confidence)?;
            stance.confidence = Some(confidence);
        }

        stance.stance = patch.stance;
        if !patch.reason_refs.is_empty() {
            stance.reason_refs.extend(patch.reason_refs);
        }

        let updated = stance.clone();
        let explanation = EpistemicExplanation::new(
            format!("claim:{}", updated.claim_id().as_str()),
            EpistemicExplanationKind::StanceUpdate,
        )
        .with_consumed_input(format!("stance:{}", updated.stance_id()))
        .with_workspace_ref(updated.workspace_id().map(|ws| ws.as_str().to_owned()))
        .with_reason_ref(updated.reason_refs().last().cloned());
        self.record_claim_explanation(updated.claim_id(), explanation);

        Ok(updated)
    }

    /// Read a stance by its stable stance ID.
    pub fn stance_by_id(&self, stance_id: &str) -> Result<&AgentStance, GraphError> {
        self.stances_by_id
            .get(stance_id)
            .ok_or_else(|| GraphError::StanceNotFound(stance_id.to_owned()))
    }

    /// Read all stance records for a claim.
    pub fn stances_by_claim(&self, claim_id: &ClaimId) -> Result<Vec<AgentStance>, GraphError> {
        self.ensure_claim_exists(claim_id)?;

        Ok(self
            .stances_by_id
            .values()
            .filter(|stance| stance.claim_id() == claim_id)
            .cloned()
            .collect())
    }

    /// Read all stance records for an agent reference.
    pub fn stances_by_agent(&self, agent_ref: &str) -> Result<Vec<AgentStance>, GraphError> {
        Ok(self
            .stances_by_id
            .values()
            .filter(|stance| stance.agent_ref() == agent_ref)
            .cloned()
            .collect())
    }

    /// Build a minimal belief-state view over an agent's stances.
    pub fn belief_state_for_agent(&self, agent_ref: &str) -> Result<BeliefState, GraphError> {
        let stances = self.stances_by_agent(agent_ref)?;
        Ok(BeliefState::new(agent_ref.to_owned(), stances))
    }

    /// Register evidence so deterministic link validation can reject unknown
    /// evidence IDs with explicit typed errors.
    pub fn register_evidence(&mut self, evidence_id: EvidenceId) {
        self.known_evidence.insert(evidence_id);
    }

    /// Create a claim at the `Candidate` lifecycle state.
    pub fn create_candidate_claim(&mut self, input: ClaimInput) -> Result<ClaimId, GraphError> {
        self.insert_claim(input, ClaimStatus::Candidate)
    }

    /// Create a claim at the `Asserted` lifecycle state.
    pub fn create_asserted_claim(&mut self, input: ClaimInput) -> Result<ClaimId, GraphError> {
        self.insert_claim(input, ClaimStatus::Asserted)
    }

    /// Read a claim by typed ID.
    pub fn claim_by_id(&self, claim_id: &ClaimId) -> Result<&Claim, GraphError> {
        self.claims
            .get(claim_id)
            .ok_or_else(|| GraphError::ClaimNotFound(claim_id.clone()))
    }

    /// Read all persisted claim support/refutation links.
    pub fn claim_links(&self) -> &[ClaimLink] {
        self.claim_links.as_slice()
    }

    /// Read decision metadata for retraction/rejection events by claim.
    pub fn claim_decisions_for_claim(
        &self,
        claim_id: &ClaimId,
    ) -> Result<&[ClaimDecision], GraphError> {
        self.ensure_claim_exists(claim_id)?;

        Ok(self
            .claim_decisions
            .get(claim_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]))
    }

    /// Retract a claim with explicit reason metadata.
    ///
    /// Retraction does not delete claim history and must remain readable.
    pub fn retract_claim(
        &mut self,
        claim_id: ClaimId,
        reason: String,
        actor_ref: Option<String>,
        session_ref: Option<String>,
    ) -> Result<Claim, GraphError> {
        self.transition_claim_with_decision(
            claim_id,
            ClaimStatus::Retracted,
            ClaimDecisionKind::Retraction,
            reason,
            actor_ref,
            session_ref,
        )
    }

    /// Reject a claim with explicit reason metadata.
    ///
    /// Rejection does not delete claim history and must remain readable.
    pub fn reject_claim(
        &mut self,
        claim_id: ClaimId,
        reason: String,
        actor_ref: Option<String>,
        session_ref: Option<String>,
    ) -> Result<Claim, GraphError> {
        self.transition_claim_with_decision(
            claim_id,
            ClaimStatus::Rejected,
            ClaimDecisionKind::Rejection,
            reason,
            actor_ref,
            session_ref,
        )
    }

    /// Attach a supporting evidence link to a target claim.
    ///
    /// Support means "there is context in favor of this claim" and does not
    /// mean the claim is automatically validated.
    pub fn attach_supporting_evidence_to_claim(
        &mut self,
        evidence_id: EvidenceId,
        target_claim_id: ClaimId,
    ) -> Result<ClaimLink, GraphError> {
        self.attach_evidence_link_to_claim(evidence_id, target_claim_id, ClaimLinkKind::Supports)
    }

    /// Attach a refuting evidence link to a target claim.
    ///
    /// Refutation means "there is context against this claim" and does not
    /// mean the claim is automatically rejected.
    pub fn attach_refuting_evidence_to_claim(
        &mut self,
        evidence_id: EvidenceId,
        target_claim_id: ClaimId,
    ) -> Result<ClaimLink, GraphError> {
        self.attach_evidence_link_to_claim(evidence_id, target_claim_id, ClaimLinkKind::Refutes)
    }

    /// Attach a supporting claim-to-claim link.
    pub fn attach_supporting_claim_to_claim(
        &mut self,
        source_claim_id: ClaimId,
        target_claim_id: ClaimId,
    ) -> Result<ClaimLink, GraphError> {
        self.attach_claim_link_to_claim(
            source_claim_id,
            target_claim_id,
            ClaimLinkKind::Supports,
            None,
        )
    }

    /// Attach a refuting claim-to-claim link.
    pub fn attach_refuting_claim_to_claim(
        &mut self,
        source_claim_id: ClaimId,
        target_claim_id: ClaimId,
    ) -> Result<ClaimLink, GraphError> {
        self.attach_claim_link_to_claim(
            source_claim_id,
            target_claim_id,
            ClaimLinkKind::Refutes,
            None,
        )
    }

    /// Mark one claim as contradicting another claim.
    ///
    /// Contradiction expresses conflict context only and does not perform final
    /// resolution policy decisions.
    pub fn attach_contradicting_claim_to_claim(
        &mut self,
        source_claim_id: ClaimId,
        target_claim_id: ClaimId,
        explanation_ref: Option<String>,
    ) -> Result<ClaimLink, GraphError> {
        if source_claim_id == target_claim_id {
            return Err(GraphError::SelfContradictionNotAllowed(source_claim_id));
        }

        self.attach_claim_link_to_claim(
            source_claim_id,
            target_claim_id,
            ClaimLinkKind::Contradicts,
            explanation_ref,
        )
    }

    /// Mark one claim as superseding another claim.
    ///
    /// Supersession expresses replacement while preserving both historical
    /// claims for append-only audit compatibility.
    pub fn attach_superseding_claim_to_claim(
        &mut self,
        source_claim_id: ClaimId,
        target_claim_id: ClaimId,
        explanation_ref: Option<String>,
    ) -> Result<ClaimLink, GraphError> {
        if source_claim_id == target_claim_id {
            return Err(GraphError::SelfSupersessionNotAllowed(source_claim_id));
        }

        self.attach_claim_link_to_claim(
            source_claim_id,
            target_claim_id,
            ClaimLinkKind::Supersedes,
            explanation_ref,
        )
    }

    fn attach_evidence_link_to_claim(
        &mut self,
        evidence_id: EvidenceId,
        target_claim_id: ClaimId,
        kind: ClaimLinkKind,
    ) -> Result<ClaimLink, GraphError> {
        if !self.known_evidence.contains(&evidence_id) {
            return Err(GraphError::EvidenceNotFound(evidence_id));
        }

        self.ensure_claim_exists(&target_claim_id)?;

        let link = ClaimLink::new(
            ClaimLinkSource::Evidence(evidence_id),
            target_claim_id,
            kind,
        );
        self.claim_links.push(link.clone());

        let explanation = EpistemicExplanation::new(
            format!("claim:{}", link.target_claim_id().as_str()),
            claim_link_kind_to_explanation_kind(link.kind()),
        )
        .with_consumed_input(claim_link_explanation_key(&link));
        self.record_link_explanation(&link, explanation.clone());
        self.record_claim_explanation(link.target_claim_id(), explanation);

        Ok(link)
    }

    fn attach_claim_link_to_claim(
        &mut self,
        source_claim_id: ClaimId,
        target_claim_id: ClaimId,
        kind: ClaimLinkKind,
        explanation_ref: Option<String>,
    ) -> Result<ClaimLink, GraphError> {
        if source_claim_id == target_claim_id {
            return Err(GraphError::InvalidClaimLink(
                "self-link claim-to-claim links are not allowed".to_owned(),
            ));
        }

        self.ensure_claim_exists(&source_claim_id)?;
        self.ensure_claim_exists(&target_claim_id)?;

        let link = ClaimLink::new(
            ClaimLinkSource::Claim(source_claim_id),
            target_claim_id,
            kind,
        )
        .with_explanation_ref(explanation_ref);
        self.claim_links.push(link.clone());

        let explanation = EpistemicExplanation::new(
            format!("claim:{}", link.target_claim_id().as_str()),
            claim_link_kind_to_explanation_kind(link.kind()),
        )
        .with_consumed_input(claim_link_explanation_key(&link))
        .with_reason_ref(link.explanation_ref().map(str::to_owned));
        self.record_link_explanation(&link, explanation.clone());
        self.record_claim_explanation(link.target_claim_id(), explanation);

        Ok(link)
    }

    fn ensure_claim_exists(&self, claim_id: &ClaimId) -> Result<(), GraphError> {
        if self.claims.contains_key(claim_id) {
            return Ok(());
        }

        Err(GraphError::ClaimNotFound(claim_id.clone()))
    }

    fn ensure_hypothesis_workspace_exists(
        &self,
        workspace_id: &HypothesisWorkspaceId,
    ) -> Result<(), GraphError> {
        if self.hypothesis_workspaces.contains_key(workspace_id) {
            return Ok(());
        }

        Err(GraphError::HypothesisWorkspaceNotFound(
            workspace_id.clone(),
        ))
    }

    fn transition_claim_with_decision(
        &mut self,
        claim_id: ClaimId,
        target_status: ClaimStatus,
        decision_kind: ClaimDecisionKind,
        reason: String,
        actor_ref: Option<String>,
        session_ref: Option<String>,
    ) -> Result<Claim, GraphError> {
        if reason.trim().is_empty() {
            return Err(match decision_kind {
                ClaimDecisionKind::Retraction => GraphError::MissingRetractionReason,
                ClaimDecisionKind::Rejection => GraphError::MissingRejectionReason,
            });
        }

        let current_claim = self
            .claims
            .get(&claim_id)
            .cloned()
            .ok_or_else(|| GraphError::ClaimNotFound(claim_id.clone()))?;

        ClaimStatus::ensure_valid_transition(current_claim.status, target_status)?;

        let next_version = current_claim.version + 1;
        let next_version_id = ClaimVersionId::new(format!(
            "claim-version--{}--{}",
            claim_id.as_str(),
            next_version
        ))?;

        let updated_claim = Claim {
            id: current_claim.id,
            version_id: next_version_id,
            version: next_version,
            status: target_status,
            statement: current_claim.statement,
            target: current_claim.target,
            confidence: current_claim.confidence,
            created_by: current_claim.created_by,
            source_refs: current_claim.source_refs,
            evidence_refs: current_claim.evidence_refs,
            workspace_id: current_claim.workspace_id,
            extraction_run_id: current_claim.extraction_run_id,
            temporal: current_claim.temporal,
        };

        self.claims.insert(claim_id.clone(), updated_claim.clone());
        self.claim_decisions
            .entry(claim_id.clone())
            .or_default()
            .push(ClaimDecision::new(
                decision_kind,
                reason,
                actor_ref,
                session_ref,
            ));

        let decision = self
            .claim_decisions
            .get(&claim_id)
            .and_then(|decisions| decisions.last())
            .cloned()
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(
                    "missing claim decision after insertion".to_owned(),
                )
            })?;

        let explanation_kind = match decision.kind() {
            ClaimDecisionKind::Retraction => EpistemicExplanationKind::Retraction,
            ClaimDecisionKind::Rejection => EpistemicExplanationKind::Rejection,
        };

        let explanation = EpistemicExplanation::new(
            format!("claim:{}", updated_claim.id().as_str()),
            explanation_kind,
        )
        .with_consumed_input(format!("decision:{:?}", decision.kind()))
        .with_actor_ref(decision.actor_ref().map(str::to_owned))
        .with_session_ref(decision.session_ref().map(str::to_owned))
        .with_workspace_ref(
            updated_claim
                .workspace_id()
                .map(|ws| ws.as_str().to_owned()),
        )
        .with_reason_ref(Some(decision.reason().to_owned()));
        self.record_claim_explanation(updated_claim.id(), explanation);

        Ok(updated_claim)
    }

    fn record_claim_explanation(&mut self, claim_id: &ClaimId, explanation: EpistemicExplanation) {
        self.claim_explanations
            .entry(claim_id.clone())
            .or_default()
            .push(explanation);
    }

    fn record_link_explanation(&mut self, link: &ClaimLink, explanation: EpistemicExplanation) {
        self.link_explanations
            .insert(claim_link_explanation_key(link), explanation);
    }

    fn insert_claim(
        &mut self,
        input: ClaimInput,
        status: ClaimStatus,
    ) -> Result<ClaimId, GraphError> {
        if self.claims.contains_key(&input.id) {
            return Err(GraphError::InvalidVersionState(format!(
                "claim already exists: {}",
                input.id.as_str()
            )));
        }

        let version = 1_u64;
        let version_id =
            ClaimVersionId::new(format!("claim-version--{}--{}", input.id.as_str(), version))?;

        let claim_id = input.id.clone();
        let claim = Claim {
            id: input.id,
            version_id,
            version,
            status,
            statement: input.statement,
            target: input.target,
            confidence: input.confidence,
            created_by: input.created_by,
            source_refs: input.source_refs,
            evidence_refs: input.evidence_refs,
            workspace_id: input.workspace_id,
            extraction_run_id: input.extraction_run_id,
            temporal: input.temporal,
        };

        self.claims.insert(claim_id.clone(), claim);

        Ok(claim_id)
    }
}
