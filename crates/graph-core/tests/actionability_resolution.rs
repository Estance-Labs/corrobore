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

#[test]
fn fabricated_support_rises_but_actionability_stays_blocked_until_grounded_verification() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        Some(1.0),
    );
    f.add(
        "source--second",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        Some(1.0),
    );
    let blocked = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(
        blocked.confidence_dimensions().evidence_sufficiency,
        Some(score(1.0))
    );
    assert_eq!(
        blocked.confidence_dimensions().actionability,
        Some(score(0.0))
    );
    assert_eq!(
        blocked.actionability().expect("gate").blockers(),
        &[ActionabilityBlocker::DeterministicVerificationMissing]
    );
    f.verifier(VerificationResult::Pass, true);
    let allowed = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert!(allowed.actionability().expect("gate").is_actionable());
    assert_eq!(
        allowed.confidence_dimensions().actionability,
        Some(score(1.0))
    );
    assert_eq!(
        f.stores
            .claims
            .claim_by_id(&f.claim)
            .expect("claim")
            .confidence(),
        allowed.display_confidence()
    );
    assert_eq!(f.stores.verdicts.verdicts_for_claim(&f.claim)[0], &blocked);
    let restored: Verdict =
        serde_json::from_value(serde_json::to_value(&allowed).expect("serialize"))
            .expect("restore");
    assert_eq!(restored, allowed);
}
#[test]
fn deterministic_failure_keeps_permission_closed_and_legacy_replay_has_no_gate() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        Some(1.0),
    );
    f.add(
        "source--second",
        None,
        ClaimLinkKind::Supports,
        Some(1.0),
        Some(1.0),
    );
    f.verifier(VerificationResult::Fail, true);
    let verdict = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(verdict.state(), VerdictState::Refuted);
    assert!(!verdict.actionability().expect("gate").is_actionable());
    assert!(f.resolve("ws-a-minimal-v1").actionability().is_none());
}
