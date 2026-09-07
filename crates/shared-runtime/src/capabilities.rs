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
//! One capability catalogue, projected by every protocol adapter.
//!
//! Module boundary: this module says what a capability *is* — its identity, its
//! effect on state, and the authorization it needs. It says nothing about how a
//! protocol carries it. There is no route, no tool name, no message envelope
//! and no protocol version here, and the type has no field to put one in, so a
//! protocol shape cannot enter the domain by accident.
//!
//! Adapters own presentation and nothing else. An adapter renders a capability
//! into its own surface — an HTTP route, an MCP tool, a plugin manifest entry —
//! and must cover exactly what the catalogue exposes to it. Effect and
//! authorization are decided once, here, so two adapters cannot disagree about
//! whether an operation writes.
//!
//! Exposure is named, not enumerated: a capability restricts itself to adapter
//! names, and an adapter that does not exist yet already has a projection.
//! Adding one requires no change to any capability.
use std::collections::BTreeSet;

use crate::*;

/// What a capability does to state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffect {
    /// Reads retained records and changes nothing.
    Read,
    /// Changes retained records.
    Mutation,
}

/// What a caller must hold before a capability runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAuthorization {
    /// Ordinary session authorization is enough.
    Session,
    /// An explicit mutation permission is required.
    MutationPermission,
    /// A recorded operator approval is required in addition.
    OperatorApproval,
}

/// One capability, defined once for every adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    id: String,
    summary: String,
    effect: CapabilityEffect,
    authorization: CapabilityAuthorization,
    runtime_operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restricted_to: Option<BTreeSet<String>>,
}

/// The complete catalogue, ordered by capability identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCatalogue {
    catalogue_version: String,
    capabilities: Vec<Capability>,
}

impl Capability {
    /// Define one capability.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a blank field.
    pub fn new(
        id: impl Into<String>,
        summary: impl Into<String>,
        effect: CapabilityEffect,
        authorization: CapabilityAuthorization,
        runtime_operation: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        let capability = Self {
            id: id.into(),
            summary: summary.into(),
            effect,
            authorization,
            runtime_operation: runtime_operation.into(),
            restricted_to: None,
        };
        if capability.id.trim().is_empty()
            || capability.summary.trim().is_empty()
            || capability.runtime_operation.trim().is_empty()
        {
            return Err(RuntimeError::InvalidAgentRuntimeInput(
                "capability requires an identity, a summary and a runtime operation".to_owned(),
            ));
        }
        Ok(capability)
    }

    /// Restrict the capability to named adapters. Unrestricted is the default,
    /// so a new adapter inherits every capability that has no reason to hide.
    pub fn restricted_to(mut self, adapters: impl IntoIterator<Item = &'static str>) -> Self {
        self.restricted_to = Some(adapters.into_iter().map(str::to_owned).collect());
        self
    }

    /// Stable capability identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the capability is for.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Effect on retained state, decided once for every adapter.
    pub fn effect(&self) -> CapabilityEffect {
        self.effect
    }

    /// Authorization the caller must hold.
    pub fn authorization(&self) -> CapabilityAuthorization {
        self.authorization
    }

    /// Runtime operation the capability dispatches to.
    pub fn runtime_operation(&self) -> &str {
        &self.runtime_operation
    }

    /// Whether the capability is exposed to one named adapter.
    pub fn is_exposed_to(&self, adapter: &str) -> bool {
        self.restricted_to
            .as_ref()
            .is_none_or(|adapters| adapters.contains(adapter))
    }

    /// Whether the capability changes retained state.
    pub fn writes(&self) -> bool {
        self.effect == CapabilityEffect::Mutation
    }
}

impl CapabilityCatalogue {
    /// The v1 catalogue.
    ///
    /// Every entry names the runtime operation it dispatches to, so an adapter
    /// never carries domain logic of its own: it renders identity, effect and
    /// authorization, and hands the call to that operation.
    pub fn v1() -> Self {
        let capabilities = [
            Capability::new(
                "claim.audit",
                "Read the retained claim audit path without recomputing a verdict.",
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                "graph.claim_audit_path",
            ),
            Capability::new(
                "cypher.read",
                "Execute a bounded read-only query.",
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                "gateway.execute",
            )
            .map(|capability| capability.restricted_to(["http"])),
            Capability::new(
                "cypher.write",
                "Execute an authorized mutation query.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "gateway.execute",
            )
            .map(|capability| capability.restricted_to(["http"])),
            Capability::new(
                "memory.consolidate",
                "Propose a consolidation, or apply an approved proposal.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::OperatorApproval,
                "memory.consolidate",
            ),
            Capability::new(
                "memory.forget",
                "Expire, tombstone or delete a memory under application semantics.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "memory.forget",
            ),
            Capability::new(
                "memory.recall",
                "Read a bounded explained working set.",
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                "memory.recall",
            ),
            Capability::new(
                "memory.relate",
                "Create or version an evidence-bearing relationship.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "memory.relate",
            ),
            Capability::new(
                "memory.remember",
                "Create or identity-upsert an evidence-bearing memory.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "memory.remember",
            ),
            Capability::new(
                "memory.trace",
                "Read the retained provenance and decision trace of a target.",
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                "memory.trace",
            ),
            Capability::new(
                "memory.update",
                "Apply an optimistic auditable patch.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "memory.update",
            ),
            Capability::new(
                "runtime.ready",
                "Report whether the runtime is ready before a protected call.",
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                "runtime.readiness",
            ),
            Capability::new(
                "stix.export",
                "Read the deterministic CTI-scoped STIX projection.",
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                "export.stix",
            ),
            Capability::new(
                "stix.import",
                "Import one STIX bundle and its optional evidence envelope.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "import.stix",
            ),
            Capability::new(
                "stix.validate",
                "Validate a STIX bundle or the current graph; a supported playbook may persist corrections.",
                CapabilityEffect::Mutation,
                CapabilityAuthorization::MutationPermission,
                "validate.stix",
            ),
        ]
        .into_iter()
        .collect::<Result<Vec<_>, RuntimeError>>()
        .expect("the v1 catalogue is a constant");
        Self {
            catalogue_version: "capabilities-v1".to_owned(),
            capabilities,
        }
    }

    /// Catalogue version, distinct from any protocol version an adapter uses.
    pub fn catalogue_version(&self) -> &str {
        &self.catalogue_version
    }

    /// Every capability, in identity order.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// One capability by identity.
    pub fn capability(&self, id: &str) -> Option<&Capability> {
        self.capabilities
            .iter()
            .find(|capability| capability.id() == id)
    }

    /// What one named adapter must expose.
    ///
    /// An adapter nobody has written yet already has a projection: exposure is
    /// a named restriction, not an enumeration, so adding an adapter changes no
    /// capability.
    pub fn adapter_view(&self, adapter: &str) -> Vec<&Capability> {
        self.capabilities
            .iter()
            .filter(|capability| capability.is_exposed_to(adapter))
            .collect()
    }

    /// Validate the catalogue's own invariants.
    ///
    /// # Errors
    /// [`RuntimeError::InvalidAgentRuntimeInput`] for a duplicate identity, an
    /// unordered catalogue, or a mutation that only requires session
    /// authorization.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        let invalid = |reason: &str| {
            Err(RuntimeError::InvalidAgentRuntimeInput(format!(
                "capability catalogue is invalid: {reason}"
            )))
        };
        if self.catalogue_version.trim().is_empty() {
            return invalid("the catalogue requires a version");
        }
        let mut previous: Option<&str> = None;
        for capability in &self.capabilities {
            if previous.is_some_and(|last| last >= capability.id()) {
                return invalid("identities must be unique and ordered");
            }
            // A write that needs nothing more than a session is how an adapter
            // ends up granting itself permission. It is refused here instead.
            if capability.writes() && capability.authorization() == CapabilityAuthorization::Session
            {
                return invalid("a mutation requires more than session authorization");
            }
            previous = Some(capability.id());
        }
        Ok(())
    }
}
