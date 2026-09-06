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
fn score(v: f64) -> Confidence {
    Confidence::new(v).expect("score")
}
fn stamp() -> BitemporalStamp {
    let t = TemporalTimestamp::new("2026-09-06T00:00:00Z").expect("time");
    BitemporalStamp::new(t.clone(), t).expect("stamp")
}
struct Fixture {
    stores: EpistemicStores,
    evidence: EvidenceRecordStore,
    claim: ClaimId,
    bindings: Vec<SourceAuthority>,
}
impl Fixture {
    fn new() -> Self {
        let mut stores = EpistemicStores::default();
        let claim = ClaimId::new("claim--aggregation").expect("id");
        stores
            .claims
            .create_asserted_claim(
                ClaimInput::new(
                    claim.clone(),
                    ClaimStatement::new("a factual claim").expect("statement"),
                    ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
                )
                .with_confidence(score(0.99)),
            )
            .expect("claim");
        Self {
            stores,
            evidence: EvidenceRecordStore::new(),
            claim,
            bindings: vec![],
        }
    }
    fn add(
        &mut self,
        id: &str,
        parent: Option<&str>,
        kind: ClaimLinkKind,
        strength: Option<f64>,
        authority: Option<f64>,
    ) {
        let source_id = SourceId::new(id).expect("id");
        let mut source = SourceInput::new(
            source_id.clone(),
            format!("https://example.test/{id}"),
            EvidenceSourceType::Document,
        );
        if let Some(parent) = parent {
            source = source.with_parent_source(SourceId::new(parent).expect("parent"));
        }
        self.stores.sources.register_source(source).expect("source");
        let observation = ObservationId::new(format!("observation--{id}")).expect("id");
        self.stores
            .observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    source_id.clone(),
                    "paraphrased article",
                    ObservationModality::Text,
                ),
                &self.stores.sources,
            )
            .expect("observation");
        self.stores.claims.register_observation(observation.clone());
        let mut link = ClaimLink::new(
            ClaimLinkSource::Observation(observation),
            self.claim.clone(),
            kind,
        )
        .with_bitemporal(stamp());
        if let Some(strength) = strength {
            link = link.with_strength(score(strength));
        }
        self.stores.claims.attach_link(link).expect("link");
        if let Some(authority) = authority {
            self.bindings.push(
                SourceAuthority::new(source_id, "test", "fact", score(authority), "authority-v1")
                    .expect("binding"),
            );
        }
    }
    fn resolve(&mut self, policy: &str) -> Verdict {
        self.stores
            .verdicts
            .register_source_authority_policy(
                SourceAuthorityPolicy::new("authority-v1", self.bindings.clone()).expect("policy"),
            )
            .expect("register");
        let inputs = ResolutionInputs::new(
            &self.stores.verifications,
            &self.evidence,
            &self.stores.observations,
            &self.stores.sources,
        )
        .with_source_authority("authority-v1", "test", "fact");
        resolve_claim_verdict(
            &mut self.stores.claims,
            &mut self.stores.verdicts,
            &inputs,
            &self.claim,
            stamp(),
            policy,
        )
        .expect("resolve");
        self.stores
            .verdicts
            .current_verdict(&self.claim)
            .expect("verdict")
            .clone()
    }
    fn verifier(&mut self, result: VerificationResult, deterministic: bool) {
        let record = VerificationRecord::new(
            VerificationRecordId::new("verification--1").expect("id"),
            "verifier.test",
            "1",
            deterministic,
            VerificationInputs::for_claim(self.claim.clone())
                .with_observation(ObservationId::new("observation--source--root").expect("id")),
            result,
            stamp(),
        );
        self.stores.verifications.append(record).expect("record");
    }
}
fn copies(n: usize) -> Fixture {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.5),
        Some(1.0),
    );
    for i in 1..n {
        f.add(
            &format!("source--copy-{i}"),
            Some("source--root"),
            ClaimLinkKind::Supports,
            Some(0.5),
            Some(1.0),
        );
    }
    f
}
#[test]
fn spike_c_ten_copies_raise_no_dimension_materially() {
    let baseline = copies(1).resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let repeated = copies(11).resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    for ((name, a), (other, b)) in baseline
        .confidence_dimensions()
        .present_values()
        .zip(repeated.confidence_dimensions().present_values())
    {
        assert_eq!(name, other);
        assert!(
            b.value() - a.value() <= 0.01,
            "{name}: {} -> {}",
            a.value(),
            b.value()
        );
    }
    assert_eq!(
        baseline.confidence_dimensions().present_values().count(),
        repeated.confidence_dimensions().present_values().count()
    );
    assert_eq!(
        repeated
            .source_independence()
            .expect("structure")
            .supporting_cluster_count(),
        1
    );
    assert_eq!(
        repeated
            .cluster_aggregation()
            .expect("explanation")
            .clusters()
            .len(),
        1
    );
}
#[test]
fn independent_clusters_outweigh_repeated_members() {
    let dependent = copies(2).resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let mut f = copies(1);
    f.add(
        "source--independent",
        None,
        ClaimLinkKind::Supports,
        Some(0.5),
        Some(1.0),
    );
    let independent = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(
        independent
            .confidence_dimensions()
            .evidence_sufficiency
            .expect("score")
            .value()
            > dependent
                .confidence_dimensions()
                .evidence_sufficiency
                .expect("score")
                .value()
                + 0.2
    );
    assert!(
        independent
            .confidence_dimensions()
            .source_independence
            .expect("score")
            .value()
            > dependent
                .confidence_dimensions()
                .source_independence
                .expect("score")
                .value()
    );
}
#[test]
fn within_cluster_increment_is_bounded_and_sublinear_at_two_five_and_ten() {
    let mut increments = vec![];
    for n in [1, 2, 5, 10] {
        let v = copies(n).resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
        let weight = v.cluster_aggregation().expect("report").clusters()[0]
            .support()
            .expect("weight");
        let increment = weight.within_cluster_increment().value();
        assert!(increment <= WITHIN_CLUSTER_INCREMENT_CAP);
        assert_eq!(weight.best_strength(), score(0.5));
        assert_eq!(weight.contributing_members(), n);
        increments.push(increment);
    }
    assert_eq!(increments[0], 0.0);
    assert!(increments[1] > 0.0);
    assert!(increments[2] > increments[1]);
    assert!(increments[3] > increments[2]);
    assert!((increments[3] - increments[2]) / 5.0 < (increments[2] - increments[1]) / 3.0);
    assert!((increments[2] - increments[1]) / 3.0 < increments[1]);
}
#[test]
fn deterministic_failure_outranks_maximal_support() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        Some(1.0),
    );
    f.verifier(VerificationResult::Fail, true);
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(
        v.confidence_dimensions().evidence_sufficiency,
        Some(score(1.0))
    );
    assert_eq!(
        v.confidence_dimensions().verifier_strength,
        Some(score(1.0))
    );
    assert_eq!(v.state(), VerdictState::Refuted);
    assert_eq!(
        v.confidence_dimensions().contradiction_load,
        Some(score(1.0))
    );
    assert_ne!(
        f.stores
            .claims
            .claim_by_id(&f.claim)
            .expect("claim")
            .status(),
        ClaimStatus::Validated
    );
}
#[test]
fn historical_policy_replays_without_retroactive_dimensions() {
    let mut f = copies(1);
    let historical = f.resolve("ws-a-minimal-v1");
    assert!(historical.cluster_aggregation().is_none());
    assert!(
        historical
            .confidence_dimensions()
            .evidence_sufficiency
            .is_none()
    );
    let new = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(new.policy_version(), CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(new.cluster_aggregation().is_some());
    assert_eq!(
        f.stores.verdicts.verdicts_for_claim(&f.claim)[0],
        &historical
    );
    f.stores = serde_json::from_value(serde_json::to_value(&f.stores).expect("serialize"))
        .expect("restore");
    let replay = f.resolve("ws-a-minimal-v1");
    assert_eq!(
        replay.confidence_dimensions(),
        historical.confidence_dimensions()
    );
    assert!(replay.cluster_aggregation().is_none());
}
#[test]
fn missing_inputs_stay_absent_and_scalar_confidence_is_not_a_fallback() {
    let v = Fixture::new().resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(v.confidence_dimensions().is_empty());
    assert_eq!(v.state(), VerdictState::Unknown);
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        None,
        Some(1.0),
    );
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(v.confidence_dimensions().evidence_sufficiency.is_none());
    assert_eq!(v.state(), VerdictState::InsufficientEvidence);
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        None,
    );
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(v.confidence_dimensions().evidence_sufficiency.is_none());
    assert!(v.confidence_dimensions().source_authority.is_none());
    assert_ne!(v.state(), VerdictState::Supported);
}
#[test]
fn authority_multiplies_cluster_contribution_once() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.8),
        Some(0.5),
    );
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let weight = v.cluster_aggregation().expect("report").clusters()[0]
        .support()
        .expect("weight");
    assert_eq!(weight.authority(), score(0.5));
    assert_eq!(weight.contribution(), score(0.4));
    assert!(
        (v.confidence_dimensions()
            .evidence_sufficiency
            .expect("score")
            .value()
            - 0.4)
            .abs()
            < 1e-12
    );
}
#[test]
fn expired_evidence_reports_zero_temporal_validity_and_no_support() {
    let mut f = copies(1);
    let earlier = TemporalTimestamp::new("2026-09-01T00:00:00Z").expect("time");
    let mut snapshot = serde_json::to_value(&f.stores.claims).expect("snapshot");
    snapshot["claim_links"][0]["bitemporal"] =
        serde_json::to_value(BitemporalStamp::new(earlier.clone(), earlier).expect("stamp"))
            .expect("stamp");
    f.stores.claims = serde_json::from_value(snapshot).expect("restore");
    f.stores
        .claims
        .close_link_validity(
            &ClaimLinkSource::Observation(
                ObservationId::new("observation--source--root").expect("id"),
            ),
            &f.claim,
            ClaimLinkKind::Supports,
            TemporalTimestamp::new("2026-09-05T00:00:00Z").expect("time"),
        )
        .expect("close");
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(
        v.confidence_dimensions().temporal_validity,
        Some(score(0.0))
    );
    assert!(v.confidence_dimensions().evidence_sufficiency.is_none());
    assert_ne!(v.state(), VerdictState::Supported);
}

#[test]
fn current_resolution_entry_point_selects_the_new_policy() {
    let mut f = copies(1);
    f.stores
        .verdicts
        .register_source_authority_policy(
            SourceAuthorityPolicy::new("authority-v1", f.bindings.clone()).expect("policy"),
        )
        .expect("register");
    let inputs = ResolutionInputs::new(
        &f.stores.verifications,
        &f.evidence,
        &f.stores.observations,
        &f.stores.sources,
    )
    .with_source_authority("authority-v1", "test", "fact");
    resolve_current_claim_verdict(
        &mut f.stores.claims,
        &mut f.stores.verdicts,
        &inputs,
        &f.claim,
        stamp(),
    )
    .expect("resolve");
    let v = f
        .stores
        .verdicts
        .current_verdict(&f.claim)
        .expect("verdict");
    assert_eq!(v.policy_version(), DEFAULT_VERDICT_POLICY_VERSION);
    assert!(v.cluster_aggregation().is_some());
    assert_eq!(
        v.confidence_dimensions().evidence_sufficiency,
        Some(score(0.5))
    );
}
#[test]
fn unweighted_or_zero_members_do_not_boost_a_cluster() {
    let baseline = copies(1).resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let mut f = copies(1);
    f.add(
        "source--unbound",
        Some("source--root"),
        ClaimLinkKind::Supports,
        Some(1.0),
        None,
    );
    f.add(
        "source--zero",
        Some("source--root"),
        ClaimLinkKind::Supports,
        Some(0.0),
        Some(1.0),
    );
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(
        v.confidence_dimensions().evidence_sufficiency,
        baseline.confidence_dimensions().evidence_sufficiency
    );
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.0),
        Some(1.0),
    );
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(
        v.confidence_dimensions().evidence_sufficiency,
        Some(score(0.0))
    );
    assert_ne!(v.state(), VerdictState::Supported);
}
#[test]
fn contradiction_load_uses_directional_cluster_scores_and_advisory_checks_do_not_override() {
    let mut f = copies(1);
    f.add(
        "source--opposition",
        None,
        ClaimLinkKind::Refutes,
        Some(0.5),
        Some(1.0),
    );
    f.verifier(VerificationResult::Pass, false);
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(v.state(), VerdictState::Mixed);
    assert_eq!(
        v.confidence_dimensions().contradiction_load,
        Some(score(0.5))
    );
    assert_eq!(
        v.confidence_dimensions().verifier_strength,
        Some(score(0.0))
    );
    // WS-D item 6 now computes permission independently of advisory support.
    assert_eq!(v.confidence_dimensions().actionability, Some(score(0.0)));
}
#[test]
fn unstamped_evidence_does_not_invent_temporal_validity() {
    let mut f = copies(1);
    let mut snapshot = serde_json::to_value(&f.stores.claims).expect("snapshot");
    snapshot["claim_links"][0]
        .as_object_mut()
        .expect("link")
        .remove("bitemporal");
    f.stores.claims = serde_json::from_value(snapshot).expect("restore");
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(v.confidence_dimensions().temporal_validity.is_none());
    assert_eq!(
        v.confidence_dimensions().evidence_sufficiency,
        Some(score(0.5))
    );
}

#[test]
fn unknown_policy_never_silently_uses_the_legacy_algorithm() {
    let mut f = copies(1);
    let before = f.stores.clone();
    let inputs = ResolutionInputs::new(
        &f.stores.verifications,
        &f.evidence,
        &f.stores.observations,
        &f.stores.sources,
    );
    assert!(
        resolve_claim_verdict(
            &mut f.stores.claims,
            &mut f.stores.verdicts,
            &inputs,
            &f.claim,
            stamp(),
            "ws-d-cluster-typo"
        )
        .is_err()
    );
    assert_eq!(f.stores, before);
}

#[test]
fn evidence_risk_reduces_independence_and_weight_without_overriding_deterministic_failure() {
    let mut f = Fixture::new();
    for name in ["source--root", "source--second", "source--third"] {
        f.add(name, None, ClaimLinkKind::Supports, Some(0.8), Some(1.0));
        f.evidence
            .create_evidence(
                EvidenceInput::new(
                    EvidenceId::new(format!("evidence--{name}")).expect("id"),
                    name,
                    "same copied factual report",
                )
                .with_source_id(SourceId::new(name).expect("id"))
                .with_observation_id(
                    ObservationId::new(format!("observation--{name}")).expect("id"),
                ),
            )
            .expect("evidence");
    }
    let before = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(f.stores.clone());
    graph.replace_evidence_store(f.evidence.clone());
    let features: Vec<_> = graph
        .evidence_store()
        .records()
        .iter()
        .map(|e| EvidenceRiskFeatures::new(e.id().clone(), "fixture-v1"))
        .collect();
    graph
        .apply_evidence_risks(
            &f.claim,
            &features,
            stamp(),
            "risk-review",
            &mut GraphTierRegistry::new(),
            &mut ImmuneResponder::new(),
        )
        .expect("risk application");
    f.stores = graph.epistemic_stores().clone();
    f.evidence = graph.evidence_store().clone();
    let after = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(
        after
            .confidence_dimensions()
            .source_independence
            .expect("dimension")
            .value()
            < before
                .confidence_dimensions()
                .source_independence
                .expect("dimension")
                .value()
    );
    assert!(
        after
            .cluster_aggregation()
            .expect("aggregation")
            .support_score()
            .expect("score")
            .value()
            < before
                .cluster_aggregation()
                .expect("aggregation")
                .support_score()
                .expect("score")
                .value()
    );
    f.verifier(VerificationResult::Fail, true);
    assert_eq!(
        f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION).state(),
        VerdictState::Refuted
    );
}
