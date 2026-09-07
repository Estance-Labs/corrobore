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
//! Integration contract for typed evidence links (Epic 0029, WS-A item 4,
//! issue #150).
//!
//! `ClaimLink` is the evidence link of ADR-0016: it gains four relation kinds
//! (`ContextFor`, `Duplicates`, `DerivedFrom`, `DependsOn`), an `Observation`
//! source, and optional governance fields (strength, authority, independence
//! cluster, bitemporal stamp) that WS-D will populate. Everything is additive:
//! links serialized before this change deserialize unchanged.
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimStore, ClaimTarget, Confidence, EpistemicExplanationKind,
    EpistemicRelationKind, EvidenceId, EvidenceSourceType, GraphError, ObservationId,
    ObservationInput, ObservationModality, ObservationStore, PropertyValue, SourceId, SourceInput,
    SourceStore, TemporalTimestamp,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn observation_id(value: &str) -> ObservationId {
    ObservationId::new(value).expect("test observation ID should be valid")
}

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("test source ID should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("test confidence should be valid")
}

fn create_asserted_claim(store: &mut ClaimStore, id: &str) -> ClaimId {
    let id = claim_id(id);
    store
        .create_asserted_claim(ClaimInput::new(
            id.clone(),
            ClaimStatement::new(format!("statement of {}", id.as_str()))
                .expect("statement should be valid"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(id.as_str(), None)),
        ))
        .expect("asserted claim should be created");
    id
}

fn observation_bound_to_source() -> (SourceStore, ObservationStore, ObservationId) {
    let mut sources = SourceStore::new();
    sources
        .register_source(SourceInput::new(
            source_id("source--report"),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .expect("source should register");
    let mut observations = ObservationStore::new();
    let id = observations
        .create_observation(
            ObservationInput::new(
                observation_id("observation--span-1"),
                source_id("source--report"),
                "APT-K-47 operated Winter Lantern.",
                ObservationModality::Text,
            ),
            &sources,
        )
        .expect("observation should be created");
    (sources, observations, id)
}

//
// Verify that the four new link kinds exist beside the four original ones,
// each with a stable token and an explanation kind, and that the epistemic
// relation vocabulary maps every link kind both ways.
#[test]
fn link_kinds_are_closed_and_aligned_with_the_relation_vocabulary() {
    assert_eq!(
        ClaimLinkKind::ALL,
        [
            ClaimLinkKind::Supports,
            ClaimLinkKind::Refutes,
            ClaimLinkKind::Contradicts,
            ClaimLinkKind::Supersedes,
            ClaimLinkKind::ContextFor,
            ClaimLinkKind::Duplicates,
            ClaimLinkKind::DerivedFrom,
            ClaimLinkKind::DependsOn,
        ]
    );
    assert_eq!(ClaimLinkKind::ContextFor.as_str(), "context_for");
    assert_eq!(ClaimLinkKind::Duplicates.as_str(), "duplicates");
    assert_eq!(ClaimLinkKind::DerivedFrom.as_str(), "derived_from");
    assert_eq!(ClaimLinkKind::DependsOn.as_str(), "depends_on");
    assert_eq!(ClaimLinkKind::Supports.as_str(), "supports");

    assert_eq!(
        ClaimLinkKind::ContextFor.explanation_kind(),
        EpistemicExplanationKind::ContextLink
    );
    assert_eq!(
        ClaimLinkKind::Duplicates.explanation_kind(),
        EpistemicExplanationKind::DuplicateLink
    );
    assert_eq!(
        ClaimLinkKind::DerivedFrom.explanation_kind(),
        EpistemicExplanationKind::DerivationLink
    );
    assert_eq!(
        ClaimLinkKind::DependsOn.explanation_kind(),
        EpistemicExplanationKind::DependencyLink
    );
    assert_eq!(
        ClaimLinkKind::Supports.explanation_kind(),
        EpistemicExplanationKind::SupportLink
    );

    for kind in ClaimLinkKind::ALL {
        let relation = EpistemicRelationKind::from(kind);
        assert_eq!(relation.claim_link_kind(), Some(kind));
        assert!(EpistemicRelationKind::ALL.contains(&relation));
    }
    assert_eq!(
        EpistemicRelationKind::ContextFor
            .canonical_relationship_type()
            .as_str(),
        "CONTEXT_FOR"
    );
    assert_eq!(
        EpistemicRelationKind::DependsOn
            .canonical_relationship_type()
            .as_str(),
        "DEPENDS_ON"
    );
    assert_eq!(EpistemicRelationKind::ALL.len(), 14);
    // Observation-to-mention containment is not an evidence link to a claim.
    assert_eq!(EpistemicRelationKind::HasMention.claim_link_kind(), None);
    // Contextual collection membership likewise never supplies factual support.
    assert_eq!(EpistemicRelationKind::HasMember.claim_link_kind(), None);
}

//
// Verify that every new kind can be attached claim-to-claim through the
// general attach path, is stored, and is explained under its own explanation
// kind.
#[test]
fn new_kinds_attach_between_claims_and_are_explained() {
    let mut store = ClaimStore::new();
    let target = create_asserted_claim(&mut store, "claim--target");
    let context = create_asserted_claim(&mut store, "claim--context");
    let duplicate = create_asserted_claim(&mut store, "claim--duplicate");
    let derived = create_asserted_claim(&mut store, "claim--derivation-source");
    let dependency = create_asserted_claim(&mut store, "claim--dependency");

    let cases = [
        (
            context,
            ClaimLinkKind::ContextFor,
            EpistemicExplanationKind::ContextLink,
        ),
        (
            duplicate,
            ClaimLinkKind::Duplicates,
            EpistemicExplanationKind::DuplicateLink,
        ),
        (
            derived,
            ClaimLinkKind::DerivedFrom,
            EpistemicExplanationKind::DerivationLink,
        ),
        (
            dependency,
            ClaimLinkKind::DependsOn,
            EpistemicExplanationKind::DependencyLink,
        ),
    ];

    for (source, kind, explanation_kind) in cases {
        let link = store
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Claim(source.clone()),
                target.clone(),
                kind,
            ))
            .expect("new kind should attach");
        assert_eq!(link.kind(), kind);
        assert!(store.claim_links().contains(&link));

        let explanation = store
            .explain_claim_link(&link)
            .expect("link should be explained");
        assert_eq!(explanation.kind(), explanation_kind);
    }

    let explanations = store
        .explain_claim(&target)
        .expect("target should carry explanations");
    assert_eq!(explanations.len(), 4);
}

//
// Verify that a claim cannot duplicate, derive from, or depend on itself,
// while `ContextFor` between a claim and itself is rejected for the same
// reason: self-links carry no epistemic information.
#[test]
fn self_links_are_rejected_for_every_claim_to_claim_kind() {
    let mut store = ClaimStore::new();
    let claim = create_asserted_claim(&mut store, "claim--self");

    for kind in [
        ClaimLinkKind::ContextFor,
        ClaimLinkKind::Duplicates,
        ClaimLinkKind::DerivedFrom,
        ClaimLinkKind::DependsOn,
        ClaimLinkKind::Supports,
    ] {
        let result = store.attach_link(ClaimLink::new(
            ClaimLinkSource::Claim(claim.clone()),
            claim.clone(),
            kind,
        ));
        assert!(
            matches!(result, Err(GraphError::InvalidClaimLink(_))),
            "{kind:?} self-link must be rejected"
        );
    }
    assert!(store.claim_links().is_empty());
}

//
// Verify that an observation can be the source of a link once registered,
// that an unregistered observation is rejected with a typed error, and that
// the link resolves back to its observation and then to its source.
#[test]
fn observation_links_resolve_to_their_source_path() {
    let (_sources, observations, observation) = observation_bound_to_source();
    let mut store = ClaimStore::new();
    let target = create_asserted_claim(&mut store, "claim--attributed");

    let unregistered = store.attach_link(ClaimLink::new(
        ClaimLinkSource::Observation(observation.clone()),
        target.clone(),
        ClaimLinkKind::Supports,
    ));
    assert!(matches!(
        unregistered,
        Err(GraphError::ObservationNotFound(id)) if id == observation
    ));

    store.register_observation(observation.clone());
    let link = store
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(observation.clone()),
            target.clone(),
            ClaimLinkKind::Supports,
        ))
        .expect("registered observation should attach");

    assert_eq!(link.source().observation_id(), Some(&observation));
    assert!(link.source().evidence_id().is_none());
    assert!(link.source().claim_id().is_none());

    let resolved = observations
        .observation_by_id(link.source().observation_id().expect("observation source"))
        .expect("observation should exist");
    assert_eq!(resolved.source_id(), &source_id("source--report"));

    let explanation = store
        .explain_claim_link(&link)
        .expect("observation link should be explained");
    assert_eq!(explanation.kind(), EpistemicExplanationKind::SupportLink);
    assert!(
        explanation
            .consumed_inputs()
            .iter()
            .any(|input| input.contains("observation:observation--span-1")),
        "explanation should name the observation source: {:?}",
        explanation.consumed_inputs()
    );
}

//
// Verify that the governance fields are stored and returned untouched, that
// strength and authority are unit-interval `Confidence` values, and that a
// bitemporal stamp rides along with the link.
#[test]
fn governance_fields_round_trip_on_the_link() {
    let mut store = ClaimStore::new();
    let target = create_asserted_claim(&mut store, "claim--governed");
    let evidence = evidence_id("evidence--governed");
    store.register_evidence(evidence.clone());

    let stamp = BitemporalStamp::new(
        timestamp("2026-08-01T00:00:00Z"),
        timestamp("2026-08-30T12:00:00Z"),
    )
    .expect("stamp should be valid");
    let link = store
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Evidence(evidence.clone()),
                target.clone(),
                ClaimLinkKind::Supports,
            )
            .with_strength(confidence(0.8))
            .with_authority(confidence(0.6))
            .with_independence_cluster("cluster--vendor-feed")
            .with_bitemporal(stamp.clone()),
        )
        .expect("governed link should attach");

    assert_eq!(link.strength(), Some(confidence(0.8)));
    assert_eq!(link.authority(), Some(confidence(0.6)));
    assert_eq!(link.independence_cluster(), Some("cluster--vendor-feed"));
    assert_eq!(link.bitemporal(), Some(&stamp));

    let stored = store
        .claim_links()
        .iter()
        .find(|candidate| candidate.source() == link.source())
        .expect("link should be stored");
    assert_eq!(stored, &link);

    let json = serde_json::to_string(&link).expect("link should serialize");
    let restored: ClaimLink = serde_json::from_str(&json).expect("link should deserialize");
    assert_eq!(restored, link);
}

//
// Verify the compatibility contract: a link without governance fields
// serializes without the new keys, and a payload captured before this change
// deserializes with every new field absent.
#[test]
fn links_without_governance_fields_keep_the_pre_change_shape() {
    let bare = ClaimLink::new(
        ClaimLinkSource::Evidence(evidence_id("evidence--bare")),
        claim_id("claim--bare"),
        ClaimLinkKind::Refutes,
    );
    let json = serde_json::to_value(&bare).expect("link should serialize");
    for key in [
        "strength",
        "authority",
        "independence_cluster",
        "bitemporal",
    ] {
        assert!(json.get(key).is_none(), "{key} must not be emitted: {json}");
    }

    let pre_change = serde_json::json!({
        "source": { "Evidence": { "value": "evidence--pre-change" } },
        "target_claim_id": { "value": "claim--pre-change" },
        "kind": "Supports",
        "explanation_ref": null
    });
    let restored: ClaimLink =
        serde_json::from_value(pre_change).expect("pre-change link should deserialize");
    assert_eq!(restored.kind(), ClaimLinkKind::Supports);
    assert!(restored.strength().is_none());
    assert!(restored.authority().is_none());
    assert!(restored.independence_cluster().is_none());
    assert!(restored.bitemporal().is_none());
    assert_eq!(
        restored.source().evidence_id(),
        Some(&evidence_id("evidence--pre-change"))
    );
}

//
// Verify the graph-facing projection: a link renders as additive, namespaced
// `evidence_link_*` properties with optional fields omitted.
#[test]
fn link_projects_to_namespaced_properties() {
    let stamp = BitemporalStamp::new(
        timestamp("2026-08-01T00:00:00Z"),
        timestamp("2026-08-30T12:00:00Z"),
    )
    .expect("stamp should be valid");
    let link = ClaimLink::new(
        ClaimLinkSource::Observation(observation_id("observation--span-1")),
        claim_id("claim--attributed"),
        ClaimLinkKind::ContextFor,
    )
    .with_strength(confidence(0.5))
    .with_independence_cluster("cluster--vendor-feed")
    .with_bitemporal(stamp);

    let properties = link.to_property_map();
    assert_eq!(
        properties.get("evidence_link_kind"),
        Some(&PropertyValue::String("context_for".to_owned()))
    );
    assert_eq!(
        properties.get("evidence_link_source_kind"),
        Some(&PropertyValue::String("observation".to_owned()))
    );
    assert_eq!(
        properties.get("evidence_link_source"),
        Some(&PropertyValue::String("observation--span-1".to_owned()))
    );
    assert_eq!(
        properties.get("evidence_link_target_claim"),
        Some(&PropertyValue::String("claim--attributed".to_owned()))
    );
    assert_eq!(
        properties.get("evidence_link_strength"),
        Some(&PropertyValue::Float(0.5))
    );
    assert_eq!(
        properties.get("evidence_link_independence_cluster"),
        Some(&PropertyValue::String("cluster--vendor-feed".to_owned()))
    );
    assert_eq!(
        properties.get("evidence_link_valid_from"),
        Some(&PropertyValue::String("2026-08-01T00:00:00Z".to_owned()))
    );
    assert_eq!(
        properties.get("evidence_link_transaction_time"),
        Some(&PropertyValue::String("2026-08-30T12:00:00Z".to_owned()))
    );
    assert!(!properties.contains_key("evidence_link_authority"));
    assert!(!properties.contains_key("evidence_link_explanation_ref"));
    assert!(
        properties
            .keys()
            .all(|key| key.starts_with("evidence_link_"))
    );
}
