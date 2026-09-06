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
use graph_core::*;
fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("weight")
}
fn stamp() -> BitemporalStamp {
    let t = TemporalTimestamp::new("2026-09-06T00:00:00Z").expect("time");
    BitemporalStamp::new(t.clone(), t).expect("stamp")
}
fn binding(version: &str, domain: &str, predicate: &str, weight: f64) -> SourceAuthority {
    SourceAuthority::new(
        SourceId::new("source--primary").expect("id"),
        domain,
        predicate,
        confidence(weight),
        version,
    )
    .expect("binding")
}
struct Fixture {
    stores: EpistemicStores,
    evidence: EvidenceRecordStore,
    claim: ClaimId,
}
impl Fixture {
    fn new(link: bool) -> Self {
        let mut stores = EpistemicStores::default();
        let source = SourceId::new("source--primary").expect("id");
        stores
            .sources
            .register_source(SourceInput::new(
                source.clone(),
                "https://example.test/primary",
                EvidenceSourceType::Document,
            ))
            .expect("source");
        let claim = ClaimId::new("claim--fact").expect("id");
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                claim.clone(),
                ClaimStatement::new("record states a fact").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
            ))
            .expect("claim");
        if link {
            let observation = ObservationId::new("observation--primary").expect("id");
            stores
                .observations
                .create_observation(
                    ObservationInput::new(
                        observation.clone(),
                        source,
                        "span",
                        ObservationModality::Text,
                    ),
                    &stores.sources,
                )
                .expect("observation");
            stores.claims.register_observation(observation.clone());
            stores
                .claims
                .attach_link(ClaimLink::new(
                    ClaimLinkSource::Observation(observation),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                ))
                .expect("link");
        }
        Self {
            stores,
            evidence: EvidenceRecordStore::new(),
            claim,
        }
    }
    fn policy(&mut self, version: &str, bindings: Vec<SourceAuthority>) {
        self.stores
            .verdicts
            .register_source_authority_policy(
                SourceAuthorityPolicy::new(version, bindings).expect("policy"),
            )
            .expect("register");
    }
    fn resolve(&mut self, version: &str, domain: &str, predicate: &str) -> Verdict {
        let inputs = ResolutionInputs::new(
            &self.stores.verifications,
            &self.evidence,
            &self.stores.observations,
            &self.stores.sources,
        )
        .with_source_authority(version, domain, predicate);
        resolve_claim_verdict(
            &mut self.stores.claims,
            &mut self.stores.verdicts,
            &inputs,
            &self.claim,
            stamp(),
            "ws-a-minimal-v1",
        )
        .expect("resolve");
        self.stores
            .verdicts
            .current_verdict(&self.claim)
            .expect("verdict")
            .clone()
    }
    fn trust(&mut self, value: f64) -> String {
        self.stores
            .claims
            .register_trust_subject("source--primary".into());
        self.stores
            .claims
            .create_trust_input(
                TrustInputInput::new(
                    TrustInputKind::SourceReliability,
                    "source--primary".into(),
                    value,
                )
                .with_provenance_ref("review--authority".into())
                .with_reason_ref("correction history".into())
                .with_claim_ref(self.claim.clone()),
            )
            .expect("trust")
    }
}
#[test]
fn authority_is_scoped_to_both_domain_and_predicate_class() {
    let mut f = Fixture::new(true);
    f.policy(
        "v1",
        vec![
            binding("v1", "legal", "factual", 0.9),
            binding("v1", "legal", "interpretive", 0.0),
            binding("v1", "medical", "factual", 0.2),
        ],
    );
    for (domain, predicate, expected) in [
        ("legal", "factual", 0.9),
        ("legal", "interpretive", 0.0),
        ("medical", "factual", 0.2),
    ] {
        let verdict = f.resolve("v1", domain, predicate);
        assert_eq!(
            verdict.confidence_dimensions().source_authority,
            Some(confidence(expected))
        );
        assert_eq!(
            verdict
                .authority_resolution()
                .expect("explanation")
                .policy_version(),
            "v1"
        );
    }
}
#[test]
fn missing_binding_is_absent_and_trust_cannot_supply_a_default() {
    let mut f = Fixture::new(true);
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.9)]);
    f.trust(1.0);
    for (domain, predicate) in [("other", "factual"), ("legal", "other")] {
        let verdict = f.resolve("v1", domain, predicate);
        assert_eq!(verdict.confidence_dimensions().source_authority, None);
        assert_eq!(
            verdict
                .authority_resolution()
                .expect("resolution")
                .sources()[0]
                .effective_weight(),
            None
        );
        assert!(
            !verdict
                .to_property_map()
                .contains_key("verdict_dimension_source_authority")
        );
    }
}
#[test]
fn trust_is_a_bounded_input_with_provenance_and_never_a_truth_decision() {
    let mut f = Fixture::new(true);
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.9)]);
    let id = f.trust(0.3);
    let verdict = f.resolve("v1", "legal", "factual");
    assert_eq!(
        verdict.confidence_dimensions().source_authority,
        Some(confidence(0.3))
    );
    assert_eq!(verdict.state(), VerdictState::Supported);
    let source = &verdict
        .authority_resolution()
        .expect("resolution")
        .sources()[0];
    assert_eq!(source.binding().expect("binding").weight(), confidence(0.9));
    assert_eq!(source.trust_inputs()[0].trust_input_id(), id);
    assert_eq!(
        source.trust_inputs()[0].provenance_ref(),
        Some("review--authority")
    );
    assert_eq!(
        source.trust_inputs()[0].reason_ref(),
        Some("correction history")
    );
    assert_eq!(
        f.stores.claims.claim_links()[0].authority(),
        Some(confidence(0.3))
    );
}
#[test]
fn high_authority_without_evidence_never_produces_support() {
    let mut f = Fixture::new(false);
    f.policy("v1", vec![binding("v1", "legal", "factual", 1.0)]);
    f.trust(1.0);
    let verdict = f.resolve("v1", "legal", "factual");
    assert_eq!(verdict.state(), VerdictState::Unknown);
    assert_eq!(verdict.confidence_dimensions().source_authority, None);
    assert!(
        verdict
            .authority_resolution()
            .expect("resolution")
            .sources()
            .is_empty()
    );
}
#[test]
fn policy_versions_are_immutable_persisted_and_replayable() {
    let mut f = Fixture::new(true);
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.9)]);
    let original = f.resolve("v1", "legal", "factual");
    f.policy("v2", vec![binding("v2", "legal", "factual", 0.4)]);
    let revised = f.resolve("v2", "legal", "factual");
    assert_eq!(
        original.confidence_dimensions().source_authority,
        Some(confidence(0.9))
    );
    assert_eq!(
        revised.confidence_dimensions().source_authority,
        Some(confidence(0.4))
    );
    assert_eq!(f.stores.verdicts.verdicts_for_claim(&f.claim)[0], &original);
    assert_eq!(f.stores.verdicts.transitions_for_claim(&f.claim).len(), 1);
    f.stores = serde_json::from_value(serde_json::to_value(&f.stores).expect("serialize"))
        .expect("restore");
    assert_eq!(
        f.resolve("v1", "legal", "factual").authority_resolution(),
        original.authority_resolution()
    );
    let conflicting =
        SourceAuthorityPolicy::new("v1", vec![binding("v1", "legal", "factual", 0.1)])
            .expect("policy");
    assert!(
        f.stores
            .verdicts
            .register_source_authority_policy(conflicting)
            .is_err()
    );
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.9)]);
}
#[test]
fn invalid_policy_shapes_are_rejected() {
    assert!(
        SourceAuthorityPolicy::new("v1", vec![binding("v2", "legal", "factual", 0.9)]).is_err()
    );
    assert!(
        SourceAuthorityPolicy::new(
            "v1",
            vec![
                binding("v1", "legal", "factual", 0.9),
                binding("v1", "legal", "factual", 0.1)
            ]
        )
        .is_err()
    );
    assert!(SourceAuthorityPolicy::new(" ", vec![]).is_err());
}

#[test]
fn unrelated_or_future_trust_inputs_do_not_cap_authority() {
    let mut f = Fixture::new(true);
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.8)]);
    f.stores
        .claims
        .register_trust_subject("source--primary".into());
    let other = ClaimId::new("claim--other").expect("id");
    f.stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            other.clone(),
            ClaimStatement::new("other claim").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("other", None)),
        ))
        .expect("claim");
    let future = TemporalTimestamp::new("2026-09-07T00:00:00Z").expect("time");
    let expired = TemporalTimestamp::new("2026-09-05T00:00:00Z").expect("time");
    for input in [
        TrustInputInput::new(
            TrustInputKind::SourceReliability,
            "source--primary".into(),
            0.1,
        )
        .with_claim_ref(other),
        TrustInputInput::new(
            TrustInputKind::ModelReliability,
            "source--primary".into(),
            0.1,
        ),
        TrustInputInput::new(
            TrustInputKind::SourceReliability,
            "source--primary".into(),
            0.1,
        )
        .with_temporal(TemporalMetadata::default().with_recorded_at(future)),
        TrustInputInput::new(
            TrustInputKind::SourceReliability,
            "source--primary".into(),
            0.1,
        )
        .with_temporal(TemporalMetadata::default().with_valid_until(expired)),
    ] {
        f.stores.claims.create_trust_input(input).expect("trust");
    }
    let verdict = f.resolve("v1", "legal", "factual");
    assert_eq!(
        verdict.confidence_dimensions().source_authority,
        Some(confidence(0.8))
    );
    assert!(
        verdict
            .authority_resolution()
            .expect("resolution")
            .sources()[0]
            .trust_inputs()
            .is_empty()
    );
}
#[test]
fn zero_binding_stays_zero_even_with_maximal_trust_and_duplicate_links() {
    let mut f = Fixture::new(true);
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.0)]);
    f.trust(1.0);
    let link = f.stores.claims.claim_links()[0].clone();
    for _ in 0..10 {
        f.stores.claims.attach_link(link.clone()).expect("link");
    }
    let verdict = f.resolve("v1", "legal", "factual");
    assert_eq!(
        verdict.confidence_dimensions().source_authority,
        Some(confidence(0.0))
    );
    assert_eq!(
        verdict
            .authority_resolution()
            .expect("resolution")
            .sources()
            .len(),
        1
    );
}
#[test]
fn registry_without_verdicts_survives_the_epistemic_snapshot() {
    let mut stores = EpistemicStores::default();
    stores
        .verdicts
        .register_source_authority_policy(
            SourceAuthorityPolicy::new("v1", vec![binding("v1", "legal", "factual", 0.9)])
                .expect("policy"),
        )
        .expect("register");
    assert!(!stores.is_empty());
    let restored: EpistemicStores =
        serde_json::from_value(serde_json::to_value(&stores).expect("serialize")).expect("restore");
    assert_eq!(stores, restored);
    assert!(restored.verdicts.source_authority_policy("v1").is_some());
}
#[test]
fn unknown_policy_version_fails_before_mutating_any_store() {
    let mut f = Fixture::new(true);
    let before = f.stores.clone();
    let inputs = ResolutionInputs::new(
        &f.stores.verifications,
        &f.evidence,
        &f.stores.observations,
        &f.stores.sources,
    )
    .with_source_authority("missing", "legal", "factual");
    assert!(
        resolve_claim_verdict(
            &mut f.stores.claims,
            &mut f.stores.verdicts,
            &inputs,
            &f.claim,
            stamp(),
            "ws-a-minimal-v1"
        )
        .is_err()
    );
    assert_eq!(f.stores, before);
}
#[test]
fn turning_authority_off_clears_the_derived_link_weight_and_dimension() {
    let mut f = Fixture::new(true);
    f.policy("v1", vec![binding("v1", "legal", "factual", 0.9)]);
    let original = f.resolve("v1", "legal", "factual");
    let inputs = ResolutionInputs::new(
        &f.stores.verifications,
        &f.evidence,
        &f.stores.observations,
        &f.stores.sources,
    );
    resolve_claim_verdict(
        &mut f.stores.claims,
        &mut f.stores.verdicts,
        &inputs,
        &f.claim,
        stamp(),
        "ws-a-minimal-v1",
    )
    .expect("resolve without authority");
    assert_eq!(f.stores.claims.claim_links()[0].authority(), None);
    let verdict = f
        .stores
        .verdicts
        .current_verdict(&f.claim)
        .expect("verdict");
    assert_eq!(verdict.confidence_dimensions().source_authority, None);
    assert!(verdict.authority_resolution().is_none());
    assert_eq!(f.stores.verdicts.verdicts_for_claim(&f.claim)[0], &original);
}
