// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Fusion back-pointers and the memory authority cap.
//!
//! Module boundary: this module derives what a consolidation retains. It reads
//! no clock, writes no graph, and decides no approval.
//!
//! Two rules give it its purpose. A fused memory enumerates every atomic origin
//! that produced it, so consolidation aggregates without erasing where an
//! interpretation came from. And its authority is a maximum over those origins,
//! each capped by the authority its own source is granted, so remembering
//! something repeatedly cannot make it more authoritative than the strongest
//! justified source behind it. That is the provenance-laundering guardrail:
//! repetition is not evidence.
//!
//! Authority requires a binding. An origin whose source has no WS-D authority
//! binding, and a fusion evaluated with no policy at all, carry no justified
//! authority rather than an assumed default.

use graph_core::{SourceAuthorityPolicy, SourceId};
use serde::{Deserialize, Serialize};

use crate::memory::ProvenanceReference;

/// The WS-D authority policy that caps a fusion, named by the caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryAuthorityPolicyRef {
    /// Registered policy version.
    pub version: String,
    /// Authority domain the cap is scoped to.
    pub authority_domain: String,
    /// Predicate class the cap is scoped to.
    pub predicate_class: String,
}

/// A scoped view of one registered authority policy.
#[derive(Clone, Copy, Debug)]
pub struct SourceAuthorityCap<'a> {
    policy: &'a SourceAuthorityPolicy,
    authority_domain: &'a str,
    predicate_class: &'a str,
}

/// One original memory as the fusion reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct FusionInput {
    /// Stable memory identity.
    pub memory_id: String,
    /// Sources the memory cites.
    pub provenance: Vec<ProvenanceReference>,
    /// Confidence the application asserted.
    pub confidence: Option<f64>,
}

/// One atomic origin of a fused memory, kept whether or not it still counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FusionOrigin {
    memory_id: String,
    source_ids: Vec<String>,
    contributed_confidence: Option<f64>,
    justified_authority: Option<f64>,
    revoked: bool,
}

/// Back-pointers and capped authority of one fused memory.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FusionLineage {
    canonical_id: String,
    origins: Vec<FusionOrigin>,
    authority: Option<f64>,
    authority_source_id: Option<String>,
    policy_version: Option<String>,
    revoked_source_ids: Vec<String>,
}

impl<'a> SourceAuthorityCap<'a> {
    /// Scope one registered policy to an authority domain and predicate class.
    pub fn new(
        policy: &'a SourceAuthorityPolicy,
        authority_domain: &'a str,
        predicate_class: &'a str,
    ) -> Self {
        Self {
            policy,
            authority_domain,
            predicate_class,
        }
    }

    /// Policy version this cap resolves against.
    pub fn policy_version(&self) -> &str {
        self.policy.version()
    }

    /// The authority a source is granted, absent when it has no binding.
    /// Absence is never replaced by a default weight.
    pub fn weight_for(&self, source_id: &str) -> Option<f64> {
        let source = SourceId::new(source_id).ok()?;
        self.policy
            .binding(&source, self.authority_domain, self.predicate_class)
            .map(|binding| binding.weight().value())
    }
}

impl FusionOrigin {
    /// Memory this origin points back to.
    pub fn memory_id(&self) -> &str {
        &self.memory_id
    }

    /// Sources the origin cites, in identity order.
    pub fn source_ids(&self) -> &[String] {
        &self.source_ids
    }

    /// Confidence the application asserted for the origin.
    pub fn contributed_confidence(&self) -> Option<f64> {
        self.contributed_confidence
    }

    /// Authority the origin justifies, capped by its own sources' bindings.
    pub fn justified_authority(&self) -> Option<f64> {
        self.justified_authority
    }

    /// Whether every source behind the origin has been revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked
    }
}

impl FusionLineage {
    /// The fused memory.
    pub fn canonical_id(&self) -> &str {
        &self.canonical_id
    }

    /// Every atomic origin in memory identity order, revoked ones included.
    pub fn origins(&self) -> &[FusionOrigin] {
        &self.origins
    }

    /// Origins that still contribute.
    pub fn active_origins(&self) -> Vec<&FusionOrigin> {
        self.origins
            .iter()
            .filter(|origin| !origin.revoked)
            .collect()
    }

    /// The strongest justified authority among active origins, absent when no
    /// active origin cites a bound source.
    pub fn authority(&self) -> Option<f64> {
        self.authority
    }

    /// The source that justifies the retained authority.
    pub fn authority_source_id(&self) -> Option<&str> {
        self.authority_source_id.as_deref()
    }

    /// Policy the cap resolved against, absent when none was named.
    pub fn policy_version(&self) -> Option<&str> {
        self.policy_version.as_deref()
    }

    /// Sources withdrawn from the interpretation.
    pub fn revoked_source_ids(&self) -> &[String] {
        &self.revoked_source_ids
    }
}

/// Derive the back-pointers and capped authority of one fusion.
///
/// Origins are ordered by memory identity so a lineage is stable whatever order
/// the originals arrive in. For each origin, every live source it cites
/// contributes `min(asserted confidence, granted authority)`, and a source
/// without a binding contributes nothing. The fused authority is the maximum of
/// those contributions over active origins: never a sum, never scaled by how
/// many origins there are, which is what stops repetition from becoming
/// authority.
///
/// A revoked source is withdrawn from the computation and stays enumerated, so
/// recomputing after a revocation never deletes an observation.
pub fn fuse_lineage(
    canonical_id: impl Into<String>,
    originals: &[FusionInput],
    cap: Option<&SourceAuthorityCap<'_>>,
    revoked_source_ids: &[String],
) -> FusionLineage {
    let mut revoked: Vec<String> = revoked_source_ids.to_vec();
    revoked.sort();
    revoked.dedup();
    let mut ordered: Vec<&FusionInput> = originals.iter().collect();
    ordered.sort_by(|left, right| left.memory_id.cmp(&right.memory_id));

    let mut origins = Vec::with_capacity(ordered.len());
    let mut strongest: Option<(f64, String)> = None;
    for input in ordered {
        let mut source_ids: Vec<String> = input
            .provenance
            .iter()
            .map(|reference| reference.source_id.clone())
            .collect();
        source_ids.sort();
        source_ids.dedup();
        let live: Vec<&String> = source_ids
            .iter()
            .filter(|source| !revoked.contains(*source))
            .collect();
        // An origin is revoked only when nothing it cites remains: one live
        // source keeps it contributing.
        let origin_revoked = !source_ids.is_empty() && live.is_empty();
        let mut justified: Option<(f64, String)> = None;
        if !origin_revoked && let Some(cap) = cap {
            for source in &live {
                let Some(granted) = cap.weight_for(source) else {
                    continue;
                };
                let contribution = input.confidence.unwrap_or(1.0).min(granted);
                if justified
                    .as_ref()
                    .is_none_or(|(current, _)| contribution > *current)
                {
                    justified = Some((contribution, (*source).clone()));
                }
            }
        }
        if let Some((contribution, source)) = justified.clone()
            && strongest
                .as_ref()
                .is_none_or(|(current, _)| contribution > *current)
        {
            strongest = Some((contribution, source));
        }
        origins.push(FusionOrigin {
            memory_id: input.memory_id.clone(),
            source_ids,
            contributed_confidence: input.confidence,
            justified_authority: justified.map(|(contribution, _)| contribution),
            revoked: origin_revoked,
        });
    }

    FusionLineage {
        canonical_id: canonical_id.into(),
        origins,
        authority: strongest.as_ref().map(|(value, _)| *value),
        authority_source_id: strongest.map(|(_, source)| source),
        policy_version: cap.map(|cap| cap.policy_version().to_owned()),
        revoked_source_ids: revoked,
    }
}
