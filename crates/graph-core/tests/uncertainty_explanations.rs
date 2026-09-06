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
fn ignorance_staleness_conflict_and_ambiguity_are_distinguished() {
    let mut f = Fixture::new();
    assert_eq!(
        f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION)
            .explanation()
            .uncertainty_kind(),
        Some(UncertaintyKind::Ignorance)
    );
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.9),
        Some(1.0),
    );
    let supported = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    assert_eq!(supported.explanation().uncertainty_kind(), None);
    f.add(
        "source--opposition",
        None,
        ClaimLinkKind::Refutes,
        Some(0.5),
        Some(1.0),
    );
    // Policies are immutable: evaluate the existing authority version, retaining
    // explicit conflict through a deterministic failure and active support.
    f.verifier(VerificationResult::Fail, true);
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
    assert_eq!(
        f.stores
            .verdicts
            .current_verdict(&f.claim)
            .expect("verdict")
            .explanation()
            .uncertainty_kind(),
        Some(UncertaintyKind::UnresolvedConflict)
    );

    let mut stale = Fixture::new();
    stale.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.9),
        Some(1.0),
    );
    let earlier = TemporalTimestamp::new("2026-09-01T00:00:00Z").expect("time");
    let end = TemporalTimestamp::new("2026-09-05T00:00:00Z").expect("time");
    let mut snapshot = serde_json::to_value(&stale.stores.claims).expect("snapshot");
    snapshot["claim_links"][0]["bitemporal"] = serde_json::to_value(
        BitemporalStamp::new(earlier.clone(), earlier)
            .expect("stamp")
            .with_valid_to(end)
            .expect("end"),
    )
    .expect("serialize");
    stale.stores.claims = serde_json::from_value(snapshot).expect("restore");
    assert_eq!(
        stale
            .resolve(CLUSTER_AGGREGATION_POLICY_VERSION)
            .explanation()
            .uncertainty_kind(),
        Some(UncertaintyKind::Staleness)
    );

    let mut ambiguous = Fixture::new();
    let root = ambiguous.claim.clone();
    let alternative = ClaimId::new("claim--alternative").expect("id");
    ambiguous
        .stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            alternative.clone(),
            ClaimStatement::new("alternative reading").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
        ))
        .expect("claim");
    ambiguous
        .stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Claim(alternative.clone()),
            root.clone(),
            ClaimLinkKind::Contradicts,
        ))
        .expect("link");
    ambiguous.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.8),
        Some(1.0),
    );
    ambiguous.claim = alternative;
    ambiguous.add(
        "source--alternative",
        None,
        ClaimLinkKind::Supports,
        Some(0.8),
        Some(1.0),
    );
    ambiguous.claim = root;
    assert_eq!(
        ambiguous
            .resolve(CLUSTER_AGGREGATION_POLICY_VERSION)
            .explanation()
            .uncertainty_kind(),
        Some(UncertaintyKind::Ambiguity)
    );
}
#[test]
fn explanations_retain_member_references_weights_and_survive_projection_and_restore() {
    let mut f = Fixture::new();
    f.add(
        "source--root",
        None,
        ClaimLinkKind::Supports,
        Some(0.8),
        Some(0.5),
    );
    f.add(
        "source--copy",
        Some("source--root"),
        ClaimLinkKind::Supports,
        Some(0.5),
        Some(0.5),
    );
    let v = f.resolve(CLUSTER_AGGREGATION_POLICY_VERSION);
    let explanation = v.explanation();
    assert_eq!(explanation.clusters().len(), 1);
    let cluster = &explanation.clusters()[0];
    assert_eq!(cluster.members().len(), 2);
    assert!(
        cluster.members()[0]
            .reference()
            .expect("reference")
            .contains("observation--source--root")
    );
    assert_eq!(cluster.support().expect("weight").authority(), score(0.5));
    assert_eq!(
        cluster.support().expect("weight").best_strength(),
        score(0.8)
    );
    let restored: Verdict =
        serde_json::from_value(serde_json::to_value(&v).expect("serialize")).expect("restore");
    assert_eq!(restored.explanation(), explanation);
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(f.stores);
    let projection = graph.epistemic_projection().expect("projection");
    let nodes = projection.list_nodes().expect("nodes");
    let node = nodes
        .iter()
        .find(|n| n.has_label("Verdict"))
        .expect("verdict");
    assert_eq!(
        node.properties().get("verdict_explanation"),
        Some(&PropertyValue::Json(
            serde_json::to_value(explanation).expect("payload")
        ))
    );
}
