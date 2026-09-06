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

fn competitor(f: &mut Fixture, name: &str, kind: ClaimLinkKind) -> ClaimId {
    let id = ClaimId::new(name).expect("id");
    f.stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            id.clone(),
            ClaimStatement::new(name).expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
        ))
        .expect("claim");
    f.stores
        .claims
        .attach_link(
            ClaimLink::new(ClaimLinkSource::Claim(id.clone()), f.claim.clone(), kind)
                .with_bitemporal(stamp()),
        )
        .expect("competing link");
    id
}
#[test]
fn independent_minority_wins_and_losers_keep_cluster_scores() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.5),
        Some(0.5),
    );
    for i in 0..10 {
        f.add(
            &format!("source--copy-{i}"),
            Some("source--root"),
            ClaimLinkKind::Supports,
            Some(0.5),
            Some(0.5),
        );
    }
    let root = f.claim.clone();
    let minority = competitor(&mut f, "claim--minority", ClaimLinkKind::Contradicts);
    f.claim = minority.clone();
    f.add(
        "source--expert",
        None,
        ClaimLinkKind::Supports,
        Some(0.9),
        Some(1.0),
    );
    f.claim = root.clone();
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let set = v.hypothesis_set().expect("set");
    assert_eq!(set.hypotheses().len(), 2);
    assert_eq!(set.hypotheses()[0].claim_id(), &minority);
    let loser = &set.hypotheses()[1];
    assert_eq!(loser.claim_id(), &root);
    assert!(loser.score().expect("score").value() < 0.3);
    assert!(loser.confidence_dimensions().evidence_sufficiency.is_some());
    assert_eq!(
        loser
            .source_independence()
            .expect("structure")
            .supporting_cluster_count(),
        1
    );
    assert_eq!(
        loser
            .cluster_aggregation()
            .expect("weights")
            .clusters()
            .len(),
        2
    );
    let restored: Verdict =
        serde_json::from_value(serde_json::to_value(&v).expect("serialize")).expect("restore");
    assert_eq!(&restored, &v);
    assert!(v.to_property_map().contains_key("verdict_hypothesis_set"));
    let count = f.stores.verdicts.len();
    assert_eq!(f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION), v);
    assert_eq!(f.stores.verdicts.len(), count);
}
#[test]
fn singleton_has_no_fabricated_score_and_historical_replay_has_no_set() {
    let mut f = Fixture::new();
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let entries = v.hypothesis_set().expect("singleton").hypotheses();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].claim_id(), &f.claim);
    assert!(entries[0].score().is_none());
    assert!(f.resolve("ws-a-minimal-v1").hypothesis_set().is_none());
}
#[test]
fn tied_alternatives_are_sorted_by_claim_id_in_both_directions() {
    let mut f = Fixture::new();
    let z = competitor(&mut f, "claim--z", ClaimLinkKind::Contradicts);
    let a = competitor(&mut f, "claim--a", ClaimLinkKind::Contradicts);
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let ids: Vec<_> = v
        .hypothesis_set()
        .expect("set")
        .hypotheses()
        .iter()
        .map(|h| h.claim_id().as_str())
        .collect();
    assert_eq!(ids, vec!["claim--a", "claim--aggregation", "claim--z"]);
    f.claim = z;
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(v.hypothesis_set().expect("set").hypotheses().len(), 2);
    assert!(
        !v.hypothesis_set()
            .expect("set")
            .hypotheses()
            .iter()
            .any(|h| h.claim_id() == &a)
    );
}
#[test]
fn competitor_changes_refresh_snapshot_without_false_state_transition() {
    let mut f = Fixture::new();
    let alternative = competitor(&mut f, "claim--alternative", ClaimLinkKind::Contradicts);
    let root = f.claim.clone();
    let before = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let transitions = f.stores.verdicts.transitions_for_claim(&root).len();
    f.claim = alternative;
    f.add(
        "source--new",
        None,
        ClaimLinkKind::Supports,
        Some(0.8),
        None,
    );
    f.claim = root.clone();
    let inputs = ResolutionInputs::new(
        &f.stores.verifications,
        &f.evidence,
        &f.stores.observations,
        &f.stores.sources,
    )
    .with_source_authority("authority-v1", "test", "fact");
    let outcome = resolve_current_claim_verdict(
        &mut f.stores.claims,
        &mut f.stores.verdicts,
        &inputs,
        &root,
        stamp(),
    )
    .expect("resolve");
    assert!(outcome.changed());
    assert!(outcome.hypothesis_set().is_some());
    assert_eq!(
        f.stores.verdicts.transitions_for_claim(&root).len(),
        transitions
    );
    assert_eq!(f.stores.verdicts.verdicts_for_claim(&root)[0], &before);
}

#[test]
fn deterministic_failure_cannot_win_despite_maximal_cluster_support() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        Some(1.0),
    );
    f.verifier(VerificationResult::Fail, true);
    let root = f.claim.clone();
    let alternative = competitor(&mut f, "claim--alternative", ClaimLinkKind::Contradicts);
    f.claim = alternative.clone();
    f.add(
        "source--alternative",
        None,
        ClaimLinkKind::Supports,
        Some(0.1),
        Some(1.0),
    );
    f.claim = root;
    let verdict = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let entries = verdict.hypothesis_set().expect("set").hypotheses();
    assert_eq!(entries[0].claim_id(), &alternative);
    assert_eq!(entries[1].state(), VerdictState::Refuted);
    assert_eq!(entries[1].score(), Some(score(0.0)));
}
#[test]
fn active_supersession_is_retained_but_future_competition_is_excluded() {
    let mut f = Fixture::new();
    let replacement = competitor(&mut f, "claim--replacement", ClaimLinkKind::Supersedes);
    let future = competitor(&mut f, "claim--future", ClaimLinkKind::Contradicts);
    let mut data = serde_json::to_value(&f.stores.claims).expect("serialize");
    let time = TemporalTimestamp::new("2026-09-10T00:00:00Z").expect("time");
    data["claim_links"][1]["bitemporal"] =
        serde_json::to_value(BitemporalStamp::new(time.clone(), time).expect("stamp"))
            .expect("serialize");
    f.stores.claims = serde_json::from_value(data).expect("restore");
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let entries = v.hypothesis_set().expect("set").hypotheses();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|h| h.claim_id() == &replacement));
    assert!(!entries.iter().any(|h| h.claim_id() == &future));
    assert_eq!(v.state(), VerdictState::Superseded);
}
#[test]
fn projection_and_persistence_retain_all_ranked_alternatives_without_resolution() {
    let mut f = Fixture::new();
    competitor(&mut f, "claim--alternative", ClaimLinkKind::Contradicts);
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let expected = serde_json::to_string(v.hypothesis_set().expect("set")).expect("serialize");
    let restored: EpistemicStores =
        serde_json::from_value(serde_json::to_value(&f.stores).expect("serialize"))
            .expect("restore");
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(restored);
    let projection = graph.epistemic_projection().expect("project");
    let nodes = projection.list_nodes().expect("nodes");
    let verdict = nodes
        .iter()
        .find(|n| n.has_label("Verdict"))
        .expect("verdict");
    assert_eq!(
        verdict.properties().get("verdict_hypothesis_set"),
        Some(&PropertyValue::String(expected))
    );
    assert_eq!(
        f.stores.verdicts.len(),
        1,
        "competitor evaluations do not write verdicts"
    );
}
