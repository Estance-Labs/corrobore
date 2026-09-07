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
//! Agent write authorization, run budgets, and a tamper-evident mutation log.
//!
//! Module boundary: this module decides whether an agent run may write, and
//! records that it did. It parses no query, executes nothing, and interprets no
//! control-plane object.
//!
//! The central rule is visible in the signature: [`authorize_agent_write`]
//! never receives the query text or the prompt. A decision that cannot see the
//! prompt cannot be argued with by one, which is what "authorization is decided
//! outside the prompt" has to mean to be worth anything. Everything the
//! decision reads — the tool, the data domain, the write permission, the egress
//! targets, the approval, the run budget — is trusted context the gateway
//! resolved before the request arrived.
//!
//! Two further rules follow. A run that has exhausted its budget is refused
//! before authorization succeeds, so a runaway run cannot leave a partial
//! privileged write behind. And every accepted mutation is appended to a
//! hash-chained log that commits to its predecessor, so an edited, removed or
//! reordered entry is detectable rather than merely unlikely.
use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::*;

/// Where an agent run may send data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressPolicy {
    /// No outbound target is permitted.
    Denied,
    /// Only these targets are permitted.
    AllowList(BTreeSet<String>),
}

/// Whether a write needs a human decision first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirement {
    /// Policy grants the write without an approval record.
    NotRequired,
    /// A grant under this approval policy must accompany the request.
    Required {
        /// Approval policy the grant must name.
        policy: String,
    },
}

/// An opaque reference to a control-plane runtime object.
///
/// Per ADR-0019 the runtime keeps its own objects: this is a link, never a
/// record. Nothing here parses it, and no query interprets it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuntimeRef {
    value: String,
}

/// The identity that signs an agent write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceIdentity {
    service: String,
    key_id: String,
}

/// A human decision that accompanies a write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalGrant {
    policy: String,
    approver: ActorId,
}

/// What an agent run may do, resolved before a request is parsed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWritePolicy {
    version: String,
    allowed_tools: BTreeSet<String>,
    allowed_data_domains: BTreeSet<String>,
    write_permitted: bool,
    egress: EgressPolicy,
    approval: ApprovalRequirement,
    budget: RunBudget,
}

/// Trusted context for one agent request. No field comes from the prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunContext {
    run_ref: RuntimeRef,
    tool_ref: RuntimeRef,
    service: ServiceIdentity,
    actor: ActorRef,
    tool: String,
    data_domain: String,
    egress_targets: BTreeSet<String>,
    approval: Option<ApprovalGrant>,
}

/// Consumption of one run, measured by the control plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    /// Model tokens consumed.
    pub tokens: u64,
    /// Cost in micro-units of the billing currency.
    pub cost_micros: u64,
    /// Tool calls made.
    pub tool_calls: u64,
    /// Wall-clock milliseconds elapsed.
    pub wall_ms: u64,
}

/// Ceilings that terminate a runaway run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBudget {
    /// Maximum model tokens.
    pub max_tokens: u64,
    /// Maximum cost in micro-units.
    pub max_cost_micros: u64,
    /// Maximum tool calls.
    pub max_tool_calls: u64,
    /// Maximum wall-clock milliseconds.
    pub max_wall_ms: u64,
}

/// The dimension that stopped a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBudgetExceeded {
    /// Exhausted dimension.
    pub dimension: String,
    /// Configured ceiling.
    pub limit: u64,
    /// Measured consumption.
    pub actual: u64,
}

/// Why a write was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWriteDenial {
    /// The tool is not in the policy.
    ToolNotAllowed(String),
    /// The data domain is not in the policy.
    DataDomainNotAllowed(String),
    /// The policy grants no write permission.
    WriteNotPermitted,
    /// An outbound target is outside the egress policy.
    EgressDenied(String),
    /// The policy requires an approval the request does not carry.
    ApprovalMissing(String),
    /// The run has spent its budget.
    BudgetExhausted(RunBudgetExceeded),
}

/// The authorization outcome, with every refusal retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWriteDecision {
    policy_version: String,
    denials: Vec<AgentWriteDenial>,
}

/// One accepted mutation, committing to its predecessor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationAuditEntry {
    sequence: usize,
    run_ref: RuntimeRef,
    tool_ref: RuntimeRef,
    service: ServiceIdentity,
    actor: ActorRef,
    tool: String,
    data_domain: String,
    mutation_count: usize,
    policy_version: String,
    previous_digest: Option<String>,
    digest: String,
}

/// Append-only hash-chained mutation log.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationAuditChain {
    entries: Vec<MutationAuditEntry>,
}

/// Where a chain stops verifying.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditTamperFinding {
    /// Sequence of the first entry that does not verify.
    pub sequence: usize,
    /// What is wrong with it.
    pub reason: String,
}

impl RuntimeRef {
    /// Wrap an opaque control-plane identity.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a blank reference.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "runtime reference must not be blank".to_owned(),
            ));
        }
        Ok(Self { value })
    }

    /// The reference, exactly as the control plane issued it.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl ServiceIdentity {
    /// Name the service and signing key behind a write.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a blank field.
    pub fn new(
        service: impl Into<String>,
        key_id: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        let identity = Self {
            service: service.into(),
            key_id: key_id.into(),
        };
        if identity.service.trim().is_empty() || identity.key_id.trim().is_empty() {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "service identity requires a service and a key".to_owned(),
            ));
        }
        Ok(identity)
    }

    /// Service that signed the write.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Key the service signed with.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

impl ApprovalGrant {
    /// Record the approval policy a human decision was taken under.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a blank policy.
    pub fn new(policy: impl Into<String>, approver: ActorId) -> Result<Self, RuntimeError> {
        let grant = Self {
            policy: policy.into(),
            approver,
        };
        if grant.policy.trim().is_empty() {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "approval grant requires a policy".to_owned(),
            ));
        }
        Ok(grant)
    }

    /// Approval policy the grant was issued under.
    pub fn policy(&self) -> &str {
        &self.policy
    }

    /// Who approved.
    pub fn approver(&self) -> &ActorId {
        &self.approver
    }
}

impl RunBudget {
    /// Ceilings that stop a run before it can spend indefinitely.
    pub fn strict_default() -> Self {
        Self {
            max_tokens: 200_000,
            max_cost_micros: 5_000_000,
            max_tool_calls: 64,
            max_wall_ms: 600_000,
        }
    }

    /// The first exhausted dimension, in declaration order.
    ///
    /// Order is fixed so a terminated run always reports the same cause for the
    /// same measurement.
    pub fn exceeded(&self, usage: &RunUsage) -> Option<RunBudgetExceeded> {
        for (dimension, limit, actual) in [
            ("tokens", self.max_tokens, usage.tokens),
            ("cost_micros", self.max_cost_micros, usage.cost_micros),
            ("tool_calls", self.max_tool_calls, usage.tool_calls),
            ("wall_ms", self.max_wall_ms, usage.wall_ms),
        ] {
            if actual > limit {
                return Some(RunBudgetExceeded {
                    dimension: dimension.to_owned(),
                    limit,
                    actual,
                });
            }
        }
        None
    }
}

impl AgentWritePolicy {
    /// Build a policy from the decisions a gateway resolved.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a blank version.
    pub fn new(
        version: impl Into<String>,
        allowed_tools: BTreeSet<String>,
        allowed_data_domains: BTreeSet<String>,
        write_permitted: bool,
        egress: EgressPolicy,
        approval: ApprovalRequirement,
        budget: RunBudget,
    ) -> Result<Self, RuntimeError> {
        let policy = Self {
            version: version.into(),
            allowed_tools,
            allowed_data_domains,
            write_permitted,
            egress,
            approval,
            budget,
        };
        if policy.version.trim().is_empty() {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "agent write policy requires a version".to_owned(),
            ));
        }
        Ok(policy)
    }

    /// A policy that reads and never writes: the safe default for an agent.
    pub fn read_only(
        version: impl Into<String>,
        tool: impl Into<String>,
        domain: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        Self::new(
            version,
            BTreeSet::from([tool.into()]),
            BTreeSet::from([domain.into()]),
            false,
            EgressPolicy::Denied,
            ApprovalRequirement::NotRequired,
            RunBudget::strict_default(),
        )
    }

    /// Policy version, retained on every decision and audit entry.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Whether the policy grants any write at all.
    pub fn write_permitted(&self) -> bool {
        self.write_permitted
    }

    /// Run ceilings this policy imposes.
    pub fn budget(&self) -> RunBudget {
        self.budget
    }
}

impl AgentRunContext {
    /// Assemble the trusted context for one agent request.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a blank tool or data domain.
    pub fn new(
        run_ref: RuntimeRef,
        tool_ref: RuntimeRef,
        service: ServiceIdentity,
        actor: ActorRef,
        tool: impl Into<String>,
        data_domain: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        let context = Self {
            run_ref,
            tool_ref,
            service,
            actor,
            tool: tool.into(),
            data_domain: data_domain.into(),
            egress_targets: BTreeSet::new(),
            approval: None,
        };
        if context.tool.trim().is_empty() || context.data_domain.trim().is_empty() {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "agent run context requires a tool and a data domain".to_owned(),
            ));
        }
        Ok(context)
    }

    /// Declare an outbound target the request would reach.
    pub fn with_egress_target(mut self, target: impl Into<String>) -> Self {
        self.egress_targets.insert(target.into());
        self
    }

    /// Attach a human approval.
    pub fn with_approval(mut self, approval: ApprovalGrant) -> Self {
        self.approval = Some(approval);
        self
    }

    /// Run this request belongs to.
    pub fn run_ref(&self) -> &RuntimeRef {
        &self.run_ref
    }

    /// Tool call this request belongs to.
    pub fn tool_ref(&self) -> &RuntimeRef {
        &self.tool_ref
    }

    /// Identity that signs the write.
    pub fn service(&self) -> &ServiceIdentity {
        &self.service
    }

    /// Actor the run acts for.
    pub fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Tool name the policy is checked against.
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Data domain the policy is checked against.
    pub fn data_domain(&self) -> &str {
        &self.data_domain
    }
}

impl AgentWriteDecision {
    /// Whether the write may proceed.
    pub fn is_allowed(&self) -> bool {
        self.denials.is_empty()
    }

    /// Every refusal, retained rather than collapsed to the first.
    pub fn denials(&self) -> &[AgentWriteDenial] {
        &self.denials
    }

    /// Policy version the decision was taken under.
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }
}

/// Decide whether an agent run may write.
///
/// The query text is deliberately not a parameter. Everything read here is
/// trusted context the gateway resolved before the request arrived, so no
/// instruction inside a prompt, a parameter or a document can reach this
/// decision. Every refusal is retained: a caller sees each unmet condition
/// instead of only the first.
///
/// The budget is checked here, before the write is authorized, which is what
/// keeps a runaway run from leaving a partial privileged write behind.
pub fn authorize_agent_write(
    policy: &AgentWritePolicy,
    context: &AgentRunContext,
    usage: &RunUsage,
) -> AgentWriteDecision {
    let mut denials = Vec::new();
    if !policy.allowed_tools.contains(&context.tool) {
        denials.push(AgentWriteDenial::ToolNotAllowed(context.tool.clone()));
    }
    if !policy.allowed_data_domains.contains(&context.data_domain) {
        denials.push(AgentWriteDenial::DataDomainNotAllowed(
            context.data_domain.clone(),
        ));
    }
    if !policy.write_permitted {
        denials.push(AgentWriteDenial::WriteNotPermitted);
    }
    for target in &context.egress_targets {
        let permitted = match &policy.egress {
            EgressPolicy::Denied => false,
            EgressPolicy::AllowList(allowed) => allowed.contains(target),
        };
        if !permitted {
            denials.push(AgentWriteDenial::EgressDenied(target.clone()));
        }
    }
    if let ApprovalRequirement::Required { policy: required } = &policy.approval {
        let granted = context
            .approval
            .as_ref()
            .is_some_and(|grant| grant.policy() == required);
        if !granted {
            denials.push(AgentWriteDenial::ApprovalMissing(required.clone()));
        }
    }
    if let Some(exceeded) = policy.budget.exceeded(usage) {
        denials.push(AgentWriteDenial::BudgetExhausted(exceeded));
    }
    AgentWriteDecision {
        policy_version: policy.version.clone(),
        denials,
    }
}

impl MutationAuditEntry {
    /// Position in the chain.
    pub fn sequence(&self) -> usize {
        self.sequence
    }

    /// Run the mutation was applied under.
    pub fn run_ref(&self) -> &RuntimeRef {
        &self.run_ref
    }

    /// Tool call the mutation was applied under.
    pub fn tool_ref(&self) -> &RuntimeRef {
        &self.tool_ref
    }

    /// Identity that signed the mutation.
    pub fn service(&self) -> &ServiceIdentity {
        &self.service
    }

    /// Actor the run acted for.
    pub fn actor(&self) -> &ActorRef {
        &self.actor
    }

    /// Tool that applied the mutation.
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Data domain the mutation touched.
    pub fn data_domain(&self) -> &str {
        &self.data_domain
    }

    /// Graph operations the mutation applied.
    pub fn mutation_count(&self) -> usize {
        self.mutation_count
    }

    /// Digest of the preceding entry, absent for the first.
    pub fn previous_digest(&self) -> Option<&str> {
        self.previous_digest.as_deref()
    }

    /// Digest committing to this entry and its predecessor.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    // Every field is committed to, separated by a byte no identifier can
    // contain, so two different entries can never hash the same material.
    fn compute_digest(&self) -> String {
        let material = [
            self.sequence.to_string(),
            self.run_ref.as_str().to_owned(),
            self.tool_ref.as_str().to_owned(),
            self.service.service().to_owned(),
            self.service.key_id().to_owned(),
            self.actor.actor_id.as_str().to_owned(),
            actor_kind_token(&self.actor.kind).to_owned(),
            self.tool.clone(),
            self.data_domain.clone(),
            self.mutation_count.to_string(),
            self.policy_version.clone(),
            self.previous_digest.clone().unwrap_or_default(),
        ]
        .join("\u{1f}");
        Sha256::digest(material.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl MutationAuditChain {
    /// Every entry, oldest first.
    pub fn entries(&self) -> &[MutationAuditEntry] {
        &self.entries
    }

    /// Digest of the newest entry, the value an external witness records.
    pub fn head_digest(&self) -> Option<&str> {
        self.entries.last().map(MutationAuditEntry::digest)
    }

    /// Append one accepted mutation, committing to the current head.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] when the decision did not allow the
    /// write: a refused write is never audited as an applied one.
    pub fn append(
        &mut self,
        context: &AgentRunContext,
        decision: &AgentWriteDecision,
        mutation_count: usize,
    ) -> Result<&MutationAuditEntry, RuntimeError> {
        if !decision.is_allowed() {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "a refused write cannot be recorded as applied".to_owned(),
            ));
        }
        let mut entry = MutationAuditEntry {
            sequence: self.entries.len(),
            run_ref: context.run_ref.clone(),
            tool_ref: context.tool_ref.clone(),
            service: context.service.clone(),
            actor: context.actor.clone(),
            tool: context.tool.clone(),
            data_domain: context.data_domain.clone(),
            mutation_count,
            policy_version: decision.policy_version.clone(),
            previous_digest: self.head_digest().map(str::to_owned),
            digest: String::new(),
        };
        entry.digest = entry.compute_digest();
        self.entries.push(entry);
        Ok(self.entries.last().expect("appended entry"))
    }

    /// Verify that no entry was edited, removed or reordered.
    ///
    /// # Errors
    /// [`AuditTamperFinding`] naming the first entry that does not verify.
    pub fn verify(&self) -> Result<(), AuditTamperFinding> {
        let mut previous: Option<&str> = None;
        for (position, entry) in self.entries.iter().enumerate() {
            if entry.sequence != position {
                return Err(AuditTamperFinding {
                    sequence: entry.sequence,
                    reason: format!("entry is out of order at position {position}"),
                });
            }
            if entry.previous_digest.as_deref() != previous {
                return Err(AuditTamperFinding {
                    sequence: entry.sequence,
                    reason: "entry does not commit to its predecessor".to_owned(),
                });
            }
            if entry.compute_digest() != entry.digest {
                return Err(AuditTamperFinding {
                    sequence: entry.sequence,
                    reason: "entry content does not match its digest".to_owned(),
                });
            }
            previous = Some(entry.digest.as_str());
        }
        Ok(())
    }
}

// A stable token per actor kind: the digest must not depend on a debug format.
fn actor_kind_token(kind: &ActorKind) -> &'static str {
    match kind {
        ActorKind::User => "user",
        ActorKind::Agent => "agent",
        ActorKind::OrchestratorAgent => "orchestrator_agent",
        ActorKind::WorkerAgent => "worker_agent",
        ActorKind::Tool => "tool",
        ActorKind::System => "system",
        ActorKind::TestFixture => "test_fixture",
    }
}
