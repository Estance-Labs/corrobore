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

/// Agent-local epistemic position independent from global claim status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StanceKind {
    /// Supports.
    Supports,
    /// Refutes.
    Refutes,
    /// Disputes.
    Disputes,
    /// Accepts.
    Accepts,
    /// Rejects.
    Rejects,
    /// Withholds judgment.
    WithholdsJudgment,
}

/// Input payload for creating a new agent stance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentStanceInput {
    pub(crate) agent_ref: String,
    pub(crate) claim_id: ClaimId,
    pub(crate) workspace_id: Option<WorkspaceId>,
    pub(crate) stance: StanceKind,
    pub(crate) confidence: Option<f64>,
    pub(crate) reason_refs: Vec<String>,
}

impl AgentStanceInput {
    /// Creates a new instance.
    pub fn new(agent_ref: String, claim_id: ClaimId, stance: StanceKind) -> Self {
        Self {
            agent_ref,
            claim_id,
            // Workspace id.
            workspace_id: None,
            stance,
            // Confidence.
            confidence: None,
            // Reason refs.
            reason_refs: Vec::new(),
        }
    }

    /// Sets the workspace id.
    pub fn with_workspace_id(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    /// Sets the confidence.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets the reason ref.
    pub fn with_reason_ref(mut self, reason_ref: String) -> Self {
        self.reason_refs.push(reason_ref);
        self
    }
}

/// Patch payload for updating an existing agent stance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentStancePatch {
    pub(crate) stance: StanceKind,
    pub(crate) confidence: Option<f64>,
    pub(crate) reason_refs: Vec<String>,
}

impl AgentStancePatch {
    /// Creates a new instance.
    pub fn new(stance: StanceKind) -> Self {
        Self {
            stance,
            // Confidence.
            confidence: None,
            // Reason refs.
            reason_refs: Vec::new(),
        }
    }

    /// Sets the confidence.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets the reason ref.
    pub fn with_reason_ref(mut self, reason_ref: String) -> Self {
        self.reason_refs.push(reason_ref);
        self
    }
}

/// First-class agent stance record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentStance {
    pub(crate) stance_id: String,
    pub(crate) agent_ref: String,
    pub(crate) claim_id: ClaimId,
    pub(crate) workspace_id: Option<WorkspaceId>,
    pub(crate) stance: StanceKind,
    pub(crate) confidence: Option<f64>,
    pub(crate) reason_refs: Vec<String>,
}

impl AgentStance {
    /// Stance id.
    pub fn stance_id(&self) -> &str {
        self.stance_id.as_str()
    }

    /// Agent ref.
    pub fn agent_ref(&self) -> &str {
        self.agent_ref.as_str()
    }

    /// Claim id.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// Workspace id.
    pub fn workspace_id(&self) -> Option<&WorkspaceId> {
        self.workspace_id.as_ref()
    }

    /// Stance.
    pub fn stance(&self) -> StanceKind {
        self.stance
    }

    /// Confidence.
    pub fn confidence(&self) -> Option<f64> {
        self.confidence
    }

    /// Reason refs.
    pub fn reason_refs(&self) -> &[String] {
        self.reason_refs.as_slice()
    }
}

/// Minimal belief-state view grouped by agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeliefState {
    pub(crate) agent_ref: String,
    pub(crate) stances: Vec<AgentStance>,
}

impl BeliefState {
    /// Creates a new instance.
    pub fn new(agent_ref: String, stances: Vec<AgentStance>) -> Self {
        Self { agent_ref, stances }
    }

    /// Agent ref.
    pub fn agent_ref(&self) -> &str {
        self.agent_ref.as_str()
    }

    /// Stances.
    pub fn stances(&self) -> &[AgentStance] {
        self.stances.as_slice()
    }
}
