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
//! Versioned authority inputs. Authority never creates evidence or decides truth.
use crate::{Confidence, GraphError, SourceId, TrustInput};
use serde::{Deserialize, Serialize};

/// Explicit authority binding, scoped to a domain and predicate class.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceAuthority {
    source_id: SourceId,
    authority_domain: String,
    predicate_class: String,
    weight: Confidence,
    policy_version: String,
}
/// Immutable, versioned registry of source authority bindings.
/// `source-reliability-cap-v1` caps each explicit weight by all applicable
/// source-reliability inputs. It never invents a weight when a binding is absent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceAuthorityPolicy {
    version: String,
    bindings: Vec<SourceAuthority>,
    trust_rule: AuthorityTrustRule,
}
/// Authority explanation for one distinct source, including unbound sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSourceAuthority {
    source_id: SourceId,
    binding: Option<SourceAuthority>,
    effective_weight: Option<Confidence>,
    trust_inputs: Vec<TrustInput>,
}
/// Exact authority context and consumed inputs retained with a verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceAuthorityResolution {
    policy_version: String,
    authority_domain: String,
    predicate_class: String,
    trust_rule: AuthorityTrustRule,
    sources: Vec<ResolvedSourceAuthority>,
}
/// Stable algorithm identifier retained in policy and verdict snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorityTrustRule {
    /// Cap explicit weights by applicable source-reliability inputs; never boost them.
    SourceReliabilityCapV1,
}

use crate::{
    ClaimId, ClaimLink, ClaimLinkKind, ClaimLinkSource, ClaimStore, ResolutionInputs,
    TrustInputKind, VerdictAsOf,
};
use std::collections::{BTreeMap, BTreeSet};
fn nonblank(value: &str) -> Result<(), GraphError> {
    if value.trim().is_empty() {
        return Err(GraphError::InvalidPropertyValue(
            "authority domain, predicate class and policy version must be nonblank".into(),
        ));
    }
    Ok(())
}
impl SourceAuthority {
    /// Construct an explicit bounded authority binding.
    /// # Errors
    /// Rejects blank scope/version fields or an invalid deserialized confidence.
    pub fn new(
        source_id: SourceId,
        domain: impl Into<String>,
        predicate: impl Into<String>,
        weight: Confidence,
        version: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let value = Self {
            source_id,
            authority_domain: domain.into(),
            predicate_class: predicate.into(),
            weight,
            policy_version: version.into(),
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), GraphError> {
        nonblank(&self.authority_domain)?;
        nonblank(&self.predicate_class)?;
        nonblank(&self.policy_version)?;
        Confidence::new(self.weight.value())?;
        Ok(())
    }
    /// Bound source identity; authority is not inherited from a parent source.
    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }
    /// Exact authority domain.
    pub fn authority_domain(&self) -> &str {
        &self.authority_domain
    }
    /// Exact predicate class supplied by the resolving policy.
    pub fn predicate_class(&self) -> &str {
        &self.predicate_class
    }
    /// Explicit policy weight before trust inputs.
    pub fn weight(&self) -> Confidence {
        self.weight
    }
    /// Version owning this binding.
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }
}
impl SourceAuthorityPolicy {
    /// Build a canonical registry using the source-reliability cap V1 rule.
    /// # Errors
    /// Rejects blank versions, duplicate keys and cross-version bindings.
    pub fn new(
        version: impl Into<String>,
        mut bindings: Vec<SourceAuthority>,
    ) -> Result<Self, GraphError> {
        bindings.sort_by(|a, b| {
            (
                a.source_id.as_str(),
                &a.authority_domain,
                &a.predicate_class,
            )
                .cmp(&(
                    b.source_id.as_str(),
                    &b.authority_domain,
                    &b.predicate_class,
                ))
        });
        let value = Self {
            version: version.into(),
            bindings,
            trust_rule: AuthorityTrustRule::SourceReliabilityCapV1,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), GraphError> {
        nonblank(&self.version)?;
        let mut keys = BTreeSet::new();
        for binding in &self.bindings {
            binding.validate()?;
            if binding.policy_version != self.version
                || !keys.insert((
                    binding.source_id.as_str(),
                    &binding.authority_domain,
                    &binding.predicate_class,
                ))
            {
                return Err(GraphError::InvalidPropertyValue("authority policy bindings must have unique source/domain/predicate keys and match the policy version".into()));
            }
        }
        Ok(())
    }
    /// Immutable policy version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// All explicit bindings in canonical order.
    pub fn bindings(&self) -> &[SourceAuthority] {
        &self.bindings
    }
    /// Exact scoped lookup. Absence is never replaced by a default weight.
    pub fn binding(
        &self,
        source: &SourceId,
        domain: &str,
        predicate: &str,
    ) -> Option<&SourceAuthority> {
        self.bindings.iter().find(|b| {
            &b.source_id == source && b.authority_domain == domain && b.predicate_class == predicate
        })
    }
    pub(crate) fn evaluate(
        &self,
        claims: &ClaimStore,
        claim: &ClaimId,
        as_of: &VerdictAsOf,
        inputs: &ResolutionInputs<'_>,
        domain: &str,
        predicate: &str,
    ) -> Result<SourceAuthorityResolution, GraphError> {
        self.validate()?;
        nonblank(domain)?;
        nonblank(predicate)?;
        let mut identities = BTreeMap::new();
        for link in claims
            .links_active_at(claim, as_of)
            .into_iter()
            .filter(|l| is_signal(l.kind()))
        {
            if let Some(source) = source_for_link(link, inputs) {
                identities.insert(source.as_str().to_owned(), source);
            }
        }
        let mut sources = Vec::new();
        for source in identities.into_values() {
            let binding = self.binding(&source, domain, predicate).cloned();
            let candidates = match claims.trust_inputs_by_subject(source.as_str()) {
                Ok(inputs) => inputs,
                Err(GraphError::TrustSubjectNotFound(_)) => Vec::new(),
                Err(error) => return Err(error),
            };
            let mut trust_inputs = Vec::new();
            for input in candidates {
                if input.kind() == TrustInputKind::SourceReliability
                    && (input.claim_refs().is_empty() || input.claim_refs().contains(claim))
                    && applicable_at(&input, as_of)?
                {
                    Confidence::new(input.value())?;
                    trust_inputs.push(input);
                }
            }
            trust_inputs.sort_by(|a, b| a.trust_input_id().cmp(b.trust_input_id()));
            let effective_weight = binding.as_ref().map(|b| {
                let weight = trust_inputs
                    .iter()
                    .fold(b.weight.value(), |weight, input| weight.min(input.value()));
                Confidence::new(weight).expect("minimum of bounded weights")
            });
            sources.push(ResolvedSourceAuthority {
                source_id: source,
                binding,
                effective_weight,
                trust_inputs,
            });
        }
        Ok(SourceAuthorityResolution {
            policy_version: self.version.clone(),
            authority_domain: domain.into(),
            predicate_class: predicate.into(),
            trust_rule: self.trust_rule,
            sources,
        })
    }
}
impl ResolvedSourceAuthority {
    /// Source whose authority was examined.
    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }
    /// Explicit binding, absent if the policy has none for this scope.
    pub fn binding(&self) -> Option<&SourceAuthority> {
        self.binding.as_ref()
    }
    /// Weight after the versioned trust-input rule; absent without a binding.
    pub fn effective_weight(&self) -> Option<Confidence> {
        self.effective_weight
    }
    /// Applicable source reliability inputs, including provenance and reasons.
    /// With no binding these are diagnostic inputs only and cannot create a weight.
    pub fn trust_inputs(&self) -> &[TrustInput] {
        &self.trust_inputs
    }
}
impl SourceAuthorityResolution {
    /// Exact authority policy version, separate from the verdict-state policy.
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }
    /// Authority domain selected by the caller's policy.
    pub fn authority_domain(&self) -> &str {
        &self.authority_domain
    }
    /// Predicate class selected by the caller's policy.
    pub fn predicate_class(&self) -> &str {
        &self.predicate_class
    }
    /// Source-by-source explanation, preserving missing bindings.
    pub fn sources(&self) -> &[ResolvedSourceAuthority] {
        &self.sources
    }
    /// Display dimension: highest known effective authority among distinct signal sources.
    /// Repetition cannot increase it. This is not cluster weighting or a truth decision.
    pub fn dimension(&self) -> Option<Confidence> {
        self.sources
            .iter()
            .filter_map(|s| s.effective_weight)
            .max_by(|a, b| a.value().total_cmp(&b.value()))
    }
    pub(crate) fn weight_for(&self, source: &SourceId) -> Option<Confidence> {
        self.sources
            .iter()
            .find(|s| &s.source_id == source)
            .and_then(|s| s.effective_weight)
    }
}
pub(crate) fn is_signal(kind: ClaimLinkKind) -> bool {
    matches!(
        kind,
        ClaimLinkKind::Supports | ClaimLinkKind::Refutes | ClaimLinkKind::Contradicts
    )
}
pub(crate) fn source_for_link(link: &ClaimLink, inputs: &ResolutionInputs<'_>) -> Option<SourceId> {
    let source = match link.source() {
        ClaimLinkSource::Observation(id) => inputs
            .observations
            .observation_by_id(id)
            .map(|o| o.source_id().clone()),
        ClaimLinkSource::Evidence(id) => inputs.evidence.evidence_by_id(id).and_then(|e| {
            e.observation_id()
                .and_then(|id| inputs.observations.observation_by_id(id))
                .map(|o| o.source_id().clone())
                .or_else(|| e.source_id().cloned())
                .or_else(|| SourceId::new(e.source_ref()).ok())
        }),
        ClaimLinkSource::Claim(_) => None,
    }?;
    inputs.sources.current_source(&source).map(|_| source)
}
fn applicable_at(input: &TrustInput, as_of: &VerdictAsOf) -> Result<bool, GraphError> {
    let parse = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value).map_err(|_| {
            GraphError::InvalidPropertyValue(format!("invalid authority trust timestamp: {value}"))
        })
    };
    let system = parse(as_of.system_time().as_str())?;
    let valid = parse(as_of.valid_time().as_str())?;
    let temporal = input.temporal();
    for time in [
        &temporal.recorded_at,
        &temporal.created_at,
        &temporal.updated_at,
    ]
    .into_iter()
    .flatten()
    {
        if parse(time)? > system {
            return Ok(false);
        }
    }
    if let Some(time) = &temporal.superseded_at
        && parse(time)? <= system
    {
        return Ok(false);
    }
    if let Some(time) = &temporal.valid_from
        && parse(time)? > valid
    {
        return Ok(false);
    }
    if let Some(time) = &temporal.valid_until
        && parse(time)? <= valid
    {
        return Ok(false);
    }
    Ok(true)
}
