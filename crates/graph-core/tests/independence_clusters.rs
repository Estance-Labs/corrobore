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

fn source(id: &str) -> SourceInput {
    SourceInput::new(
        SourceId::new(id).expect("id"),
        format!("https://example.test/{id}"),
        EvidenceSourceType::Document,
    )
}
fn time() -> BitemporalStamp {
    let t = TemporalTimestamp::new("2026-09-06T00:00:00Z").expect("time");
    BitemporalStamp::new(t.clone(), t).expect("stamp")
}
struct Fixture {
    stores: EpistemicStores,
    evidence: EvidenceRecordStore,
    claim: ClaimId,
}
impl Fixture {
    fn new() -> Self {
        let mut stores = EpistemicStores::default();
        let claim = ClaimId::new("claim--cluster").expect("id");
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                claim.clone(),
                ClaimStatement::new("A supports B").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
            ))
            .expect("claim");
        Self {
            stores,
            evidence: EvidenceRecordStore::new(),
            claim,
        }
    }
    fn add(&mut self, input: SourceInput, id: &str) {
        self.stores.sources.register_source(input).expect("source");
        let observation = ObservationId::new(format!("observation--{id}")).expect("id");
        self.stores
            .observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    SourceId::new(id).expect("id"),
                    "payload",
                    ObservationModality::Text,
                ),
                &self.stores.sources,
            )
            .expect("observation");
        self.stores.claims.register_observation(observation.clone());
        self.stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                self.claim.clone(),
                ClaimLinkKind::Supports,
            ))
            .expect("link");
    }
    fn resolve(&mut self) -> SourceIndependence {
        resolve_claim_verdict(
            &mut self.stores.claims,
            &mut self.stores.verdicts,
            &ResolutionInputs::new(
                &self.stores.verifications,
                &self.evidence,
                &self.stores.observations,
                &self.stores.sources,
            ),
            &self.claim,
            time(),
            "ws-a-minimal-v1",
        )
        .expect("resolve");
        self.stores
            .verdicts
            .current_verdict(&self.claim)
            .expect("verdict")
            .source_independence()
            .expect("structure")
            .clone()
    }
}
fn assert_pair(a: SourceInput, b: SourceInput, signal: DependencySignal) {
    let mut f = Fixture::new();
    f.add(a, "source--a");
    f.add(b, "source--b");
    let report = f.resolve();
    assert_eq!(report.supporting_cluster_count(), 1);
    assert!(
        report.clusters()[0]
            .reasons()
            .iter()
            .any(|r| r.signal() == signal)
    );
    for (index, link) in f.stores.claims.claim_links().iter().enumerate() {
        assert_eq!(
            link.independence_cluster(),
            Some(report.cluster_for_link(index).expect("member").id())
        );
    }
    let restored: EpistemicStores =
        serde_json::from_value(serde_json::to_value(&f.stores).expect("serialize"))
            .expect("restore");
    assert_eq!(
        restored
            .verdicts
            .current_verdict(&f.claim)
            .expect("verdict")
            .source_independence(),
        Some(&report)
    );
    assert_eq!(f.resolve(), report);
}
#[test]
fn shared_publisher_clusters() {
    assert_pair(
        source("source--a").with_publisher("Publisher"),
        source("source--b").with_publisher("Publisher"),
        DependencySignal::SharedPublisher,
    );
}
#[test]
fn ten_syndicated_copies_report_one_supporting_cluster() {
    let mut f = Fixture::new();
    f.add(source("source--root"), "source--root");
    for i in 0..10 {
        let id = format!("source--copy-{i}");
        f.add(
            source(&id).with_parent_source(SourceId::new("source--root").expect("id")),
            &id,
        );
    }
    let report = f.resolve();
    assert_eq!(report.supporting_cluster_count(), 1);
    assert_eq!(report.clusters()[0].members().len(), 11);
    assert!(
        report.clusters()[0]
            .reasons()
            .iter()
            .any(|r| r.signal() == DependencySignal::Syndication)
    );
}
#[test]
fn unrelated_sources_remain_separate() {
    let mut f = Fixture::new();
    f.add(source("source--a").with_publisher("A"), "source--a");
    f.add(source("source--b").with_publisher("B"), "source--b");
    assert_eq!(f.resolve().supporting_cluster_count(), 2);
}
#[test]
fn unknown_independence_is_an_explicit_singleton_not_established_independence() {
    let mut f = Fixture::new();
    for id in ["claim--unknown-a", "claim--unknown-b"] {
        let id = ClaimId::new(id).expect("id");
        f.stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                id.clone(),
                ClaimStatement::new("unknown").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("unknown", None)),
            ))
            .expect("claim");
        f.stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Claim(id),
                f.claim.clone(),
                ClaimLinkKind::Supports,
            ))
            .expect("link");
    }
    let report = f.resolve();
    assert_eq!(report.supporting_cluster_count(), 2);
    assert_eq!(report.unknown_cluster_count(), 2);
    assert!(report.clusters().iter().all(|c| {
        c.reasons()
            .iter()
            .any(|r| r.signal() == DependencySignal::UnknownIndependence)
    }));
}
#[test]
fn shared_upstream_citation_clusters() {
    let signals = SourceDependencySignals {
        upstream_citations: vec!["https://upstream.test/article".into()],
        ..Default::default()
    };
    assert_pair(
        source("source--a").with_dependency_signals(signals.clone()),
        source("source--b").with_dependency_signals(signals),
        DependencySignal::SharedUpstreamCitation,
    );
}
#[test]
fn identical_artifact_under_two_source_identities_clusters() {
    assert_pair(
        source("source--a").with_artifact_sha256("a".repeat(64)),
        source("source--b").with_artifact_sha256("a".repeat(64)),
        DependencySignal::IdenticalArtifact,
    );
}
#[test]
fn near_identical_artifact_requires_and_records_an_explicit_reason() {
    let mut f = Fixture::new();
    f.add(
        source("source--a").with_artifact_sha256("a".repeat(64)),
        "source--a",
    );
    f.add(
        source("source--b").with_artifact_sha256(format!("{}b", "a".repeat(63))),
        "source--b",
    );
    assert_eq!(f.resolve().supporting_cluster_count(), 2);
    let signals = SourceDependencySignals {
        near_duplicate_artifacts: vec![NearDuplicateArtifact {
            sha256: "a".repeat(64),
            reason: "review--42: same article with corrected punctuation".into(),
        }],
        ..Default::default()
    };
    assert_pair(
        source("source--a").with_artifact_sha256("a".repeat(64)),
        source("source--b")
            .with_artifact_sha256("b".repeat(64))
            .with_dependency_signals(signals),
        DependencySignal::NearDuplicateArtifact,
    );
}
#[test]
fn shared_extraction_run_clusters() {
    let signals = SourceDependencySignals {
        extraction_run: Some("run--1".into()),
        ..Default::default()
    };
    assert_pair(
        source("source--a").with_dependency_signals(signals.clone()),
        source("source--b").with_dependency_signals(signals),
        DependencySignal::SharedExtractionRun,
    );
}
#[test]
fn shared_model_pipeline_clusters() {
    let signals = SourceDependencySignals {
        model_pipeline: Some("extractor--v1/model--v2".into()),
        ..Default::default()
    };
    assert_pair(
        source("source--a").with_dependency_signals(signals.clone()),
        source("source--b").with_dependency_signals(signals),
        DependencySignal::SharedModelPipeline,
    );
}
#[test]
fn blank_near_duplicate_reason_is_rejected() {
    let signals = SourceDependencySignals {
        near_duplicate_artifacts: vec![NearDuplicateArtifact {
            sha256: "a".repeat(64),
            reason: " ".into(),
        }],
        ..Default::default()
    };
    assert!(
        SourceStore::new()
            .register_source(source("source--a").with_dependency_signals(signals))
            .is_err()
    );
}

#[test]
fn dependencies_join_transitively_and_are_stable_when_resolved_again() {
    let mut f = Fixture::new();
    f.add(source("source--a").with_publisher("publisher"), "source--a");
    f.add(
        source("source--b")
            .with_publisher("publisher")
            .with_artifact_sha256("a".repeat(64)),
        "source--b",
    );
    f.add(
        source("source--c").with_artifact_sha256("a".repeat(64)),
        "source--c",
    );
    let report = f.resolve();
    assert_eq!(report.supporting_cluster_count(), 1);
    assert_eq!(report.clusters()[0].members().len(), 3);
    assert_eq!(f.resolve(), report);
    assert_eq!(f.stores.verdicts.len(), 1);
}
#[test]
fn source_structure_changes_append_a_snapshot_without_a_false_state_transition() {
    let mut f = Fixture::new();
    f.add(source("source--a"), "source--a");
    let before = f.resolve();
    let transitions = f.stores.verdicts.transitions_for_claim(&f.claim).len();
    f.add(source("source--b"), "source--b");
    let after = f.resolve();
    assert_eq!(before.supporting_cluster_count(), 1);
    assert_eq!(after.supporting_cluster_count(), 2);
    assert_eq!(
        f.stores.verdicts.verdicts_for_claim(&f.claim)[0].source_independence(),
        Some(&before)
    );
    assert_eq!(
        f.stores.verdicts.transitions_for_claim(&f.claim).len(),
        transitions
    );
    let projected = f
        .stores
        .verdicts
        .current_verdict(&f.claim)
        .expect("verdict")
        .to_property_map();
    assert_eq!(
        projected.get("verdict_source_independence_supporting_clusters"),
        Some(&PropertyValue::Integer(2))
    );
    assert!(
        f.stores
            .verdicts
            .current_verdict(&f.claim)
            .expect("verdict")
            .confidence_dimensions()
            .source_independence
            .is_none()
    );
}
#[test]
fn existing_evidence_extraction_metadata_is_consumed() {
    for use_run in [true, false] {
        let mut f = Fixture::new();
        for id in ["source--a", "source--b"] {
            f.add(source(id), id);
            let evidence_id = EvidenceId::new(format!("evidence--{id}")).expect("id");
            let mut input = EvidenceInput::new(evidence_id, id, "span")
                .with_observation_id(ObservationId::new(format!("observation--{id}")).expect("id"));
            input = if use_run {
                input.with_extraction_run_id(ExtractionRunId::new("run--shared").expect("run"))
            } else {
                input
                    .with_extractor_id("extractor")
                    .with_model_version("model--v1")
            };
            f.evidence.create_evidence(input).expect("evidence");
        }
        let report = f.resolve();
        assert_eq!(report.supporting_cluster_count(), 1);
        assert!(report.clusters()[0].reasons().iter().any(|r| r.signal()
            == if use_run {
                DependencySignal::SharedExtractionRun
            } else {
                DependencySignal::SharedModelPipeline
            }));
    }
}
#[test]
fn expired_links_do_not_bridge_active_components() {
    let mut f = Fixture::new();
    f.add(source("source--a").with_publisher("publisher"), "source--a");
    f.add(
        source("source--b")
            .with_publisher("publisher")
            .with_artifact_sha256("a".repeat(64)),
        "source--b",
    );
    f.add(
        source("source--c").with_artifact_sha256("a".repeat(64)),
        "source--c",
    );
    // Rebuild the claim store snapshot with the bridge link valid only in the past.
    let mut value = serde_json::to_value(&f.stores.claims).expect("snapshot");
    let begin = TemporalTimestamp::new("2026-09-01T00:00:00Z").expect("time");
    let stamp = BitemporalStamp::new(begin.clone(), begin).expect("stamp");
    value["claim_links"][1]["bitemporal"] = serde_json::to_value(stamp).expect("stamp");
    f.stores.claims = serde_json::from_value(value).expect("restore");
    f.stores
        .claims
        .close_link_validity(
            &ClaimLinkSource::Observation(
                ObservationId::new("observation--source--b").expect("id"),
            ),
            &f.claim,
            ClaimLinkKind::Supports,
            TemporalTimestamp::new("2026-09-02T00:00:00Z").expect("time"),
        )
        .expect("close");
    let report = f.resolve();
    assert_eq!(report.supporting_cluster_count(), 2);
    assert!(report.cluster_for_link(1).is_none());
}

#[test]
fn legacy_evidence_links_follow_source_ancestry_and_explain_their_membership() {
    let mut f = Fixture::new();
    f.stores
        .sources
        .register_source(source("source--root"))
        .expect("root");
    for i in 0..10 {
        let id = format!("source--copy-{i}");
        let source_id = SourceId::new(&id).expect("id");
        f.stores
            .sources
            .register_source(
                source(&id).with_parent_source(SourceId::new("source--root").expect("parent")),
            )
            .expect("source");
        let evidence_id = EvidenceId::new(format!("evidence--{i}")).expect("id");
        f.evidence
            .create_evidence(
                EvidenceInput::new(evidence_id.clone(), &id, "copy").with_source_id(source_id),
            )
            .expect("evidence");
        f.stores.claims.register_evidence(evidence_id.clone());
        f.stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Evidence(evidence_id),
                f.claim.clone(),
                ClaimLinkKind::Supports,
            ))
            .expect("link");
    }
    let report = f.resolve();
    assert_eq!(report.supporting_cluster_count(), 1);
    assert_eq!(report.clusters()[0].members().len(), 10);
    assert!(
        report.clusters()[0]
            .reasons()
            .iter()
            .all(|r| r.signal() == DependencySignal::Syndication
                && r.value().contains("source--root"))
    );
}
