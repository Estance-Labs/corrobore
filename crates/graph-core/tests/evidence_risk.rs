#![allow(clippy::unwrap_used)]
use graph_core::*;
fn id(i: usize) -> EvidenceId {
    EvidenceId::new(format!("e{i}")).unwrap()
}
fn stamp() -> BitemporalStamp {
    let t = TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap();
    BitemporalStamp::new(t.clone(), t).unwrap()
}
fn fixture(kind: Option<EvidenceRiskSignal>) -> (Graph, ClaimId, Vec<EvidenceRiskFeatures>) {
    let mut graph = Graph::new();
    let claim = ClaimId::new("claim-risk").unwrap();
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("factual proposition").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
        ))
        .unwrap();
    let texts = [
        "alpha river crossed north",
        "beta mountain rain south",
        "gamma ocean calm west",
    ];
    let mut features = Vec::new();
    for (i, text) in texts.into_iter().enumerate() {
        let text = if kind == Some(EvidenceRiskSignal::LexicalDuplication) && i == 1 {
            "ALPHA river crossed north!"
        } else {
            text
        };
        graph
            .create_evidence(EvidenceInput::new(id(i), format!("source-{i}"), text))
            .unwrap();
        let claims = &mut graph.epistemic_stores_mut().claims;
        claims.register_evidence(id(i));
        claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Evidence(id(i)),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                )
                .with_strength(Confidence::new(0.8).unwrap()),
            )
            .unwrap();
        let mut f = EvidenceRiskFeatures::new(id(i), "fixture-v1");
        match kind {
            Some(EvidenceRiskSignal::SemanticDuplication) if i < 2 => {
                f.embedding = Some(vec![1.0, i as f64 * 0.01]);
                f.embedding_model = Some("test-model-v1".into());
            }
            Some(EvidenceRiskSignal::SharedInfrastructure) if i < 2 => {
                f.infrastructure = vec!["origin:shared-server".into()]
            }
            Some(EvidenceRiskSignal::SharedUpstreamCitation) if i < 2 => {
                f.upstream_citations = vec!["urn:primary-report".into()]
            }
            Some(EvidenceRiskSignal::TemporalBurst) => f.publication_seconds = Some(10 + i as i64),
            Some(EvidenceRiskSignal::GenerationFingerprint) if i < 2 => {
                f.generation_fingerprint = Some("watermark:fixture-unique".into())
            }
            Some(EvidenceRiskSignal::EmbeddingGeometryAnomaly) => {
                f.embedding = Some([vec![100.0, 0.0], vec![0.0, 1.0], vec![-1.0, 0.0]][i].clone());
                f.embedding_model = Some("test-model-v1".into());
            }
            _ => {}
        }
        features.push(f);
    }
    (graph, claim, features)
}
#[test]
fn each_signal_has_an_isolated_fixture_and_explanation() {
    for signal in EvidenceRiskSignal::ALL {
        let (graph, claim, features) = fixture(Some(signal));
        let findings = detect_evidence_risks(&graph, &claim, &features).unwrap();
        assert!(!findings.is_empty(), "{signal:?}");
        assert!(
            findings
                .iter()
                .all(|f| f.signal == signal && !f.reason.is_empty()),
            "{findings:?}"
        );
    }
}
#[test]
fn benign_and_missing_features_do_not_imply_risk() {
    let (graph, claim, features) = fixture(None);
    assert!(
        detect_evidence_risks(&graph, &claim, &features)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn quarantine_and_dependency_reasons_survive_audit_and_native_round_trip() {
    let (mut graph, claim, features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    let mut tiers = GraphTierRegistry::new();
    let mut immune = ImmuneResponder::new();
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "risk-review",
            &mut tiers,
            &mut immune,
        )
        .unwrap();
    assert_eq!(
        tiers.tier_of(&TierRecordRef::Evidence(id(0))),
        GraphTier::Quarantine
    );
    assert_eq!(
        tiers.tier_of(&TierRecordRef::Evidence(id(1))),
        GraphTier::Quarantine
    );
    assert_eq!(
        tiers.tier_of(&TierRecordRef::Evidence(id(2))),
        GraphTier::Canonical
    );
    assert_eq!(graph.evidence_count(), 3);
    let audit = graph.claim_audit_path(&claim).unwrap();
    assert!(audit.to_string().contains("LexicalDuplication"));
    assert!(audit.to_string().contains("Quarantine"));
    let restored = Graph::from_memory_json(&graph.export_memory_json().unwrap()).unwrap();
    assert_eq!(restored.claim_audit_path(&claim).unwrap(), audit);
    let count = immune.audit().len();
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "risk-review",
            &mut tiers,
            &mut immune,
        )
        .unwrap();
    assert_eq!(immune.audit().len(), count);
}
#[test]
fn invalid_batch_is_atomic() {
    let (mut graph, claim, mut features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    features[2].embedding = Some(vec![f64::NAN]);
    let before = graph.export_memory_json().unwrap();
    let mut tiers = GraphTierRegistry::new();
    let mut immune = ImmuneResponder::new();
    assert!(
        graph
            .apply_evidence_risks(&claim, &features, stamp(), "actor", &mut tiers, &mut immune)
            .is_err()
    );
    assert!(immune.audit().is_empty());
    assert_eq!(graph.export_memory_json().unwrap(), before);
}

#[test]
fn quarantine_covers_existing_dependency_members_outside_the_detection_batch() {
    let (mut graph, claim, features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    graph
        .create_evidence(EvidenceInput::new(
            id(3),
            "source-0",
            "different contextual material from same source",
        ))
        .unwrap();
    let claims = &mut graph.epistemic_stores_mut().claims;
    claims.register_evidence(id(3));
    claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(id(3)),
            claim.clone(),
            ClaimLinkKind::Supports,
        ))
        .unwrap();
    let mut tiers = GraphTierRegistry::new();
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut tiers,
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    assert_eq!(
        tiers.tier_of(&TierRecordRef::Evidence(id(3))),
        GraphTier::Quarantine
    );
    assert_eq!(graph.evidence_count(), 4);
}

#[test]
fn risk_follows_reused_evidence_and_scoped_archive_keeps_its_provenance() {
    let (mut graph, claim, features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut GraphTierRegistry::new(),
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    // Re-ingestion of identical immutable content must remain idempotent.
    graph
        .create_evidence(EvidenceInput::new(
            id(0),
            "source-0",
            "alpha river crossed north",
        ))
        .unwrap();
    let other = ClaimId::new("reused-claim").unwrap();
    let stores = graph.epistemic_stores_mut();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            other.clone(),
            ClaimStatement::new("other proposition").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("other", None)),
        ))
        .unwrap();
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(id(0)),
            other.clone(),
            ClaimLinkKind::Supports,
        ))
        .unwrap();
    let evidence = graph.evidence_store().clone();
    let stores = graph.epistemic_stores_mut();
    let at = VerdictAsOf::new(stamp().valid_from, stamp().transaction_time);
    let structure = stores
        .claims
        .assign_independence_clusters(
            &other,
            &at,
            &evidence,
            &stores.observations,
            &stores.sources,
        )
        .unwrap();
    assert!(structure.clusters()[0].has_evidence_risk());
    assert!(
        structure.clusters()[0]
            .reasons()
            .iter()
            .any(|r| r.signal() == DependencySignal::EvidenceRisk)
    );
    let audit = graph.claim_audit_path(&other).unwrap();
    let restored = Graph::from_claim_audit_archive(
        &graph
            .export_claim_audit_archive(std::slice::from_ref(&other))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(restored.claim_audit_path(&other).unwrap(), audit);
    assert!(restored.evidence_by_id(&id(1)).is_some());
    assert!(restored.evidence_by_id(&id(2)).is_none());
}

#[test]
fn fully_measured_benign_fixture_fires_no_signal() {
    let (mut graph, claim, mut features) = fixture(None);
    for (i, f) in features.iter_mut().enumerate() {
        let mut vector = vec![0.0; 3];
        vector[i] = 1.0;
        f.embedding = Some(vector);
        f.embedding_model = Some("fixture-space-v1".into());
        f.infrastructure = vec![format!("origin:{i}")];
        f.upstream_citations = vec![format!("citation:{i}")];
        f.publication_seconds = Some(i as i64 * 600);
        f.generation_fingerprint = Some(format!("watermark:{i}"));
    }
    assert!(
        detect_evidence_risks(&graph, &claim, &features)
            .unwrap()
            .is_empty()
    );
    let before = graph.export_memory_json().unwrap();
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut GraphTierRegistry::new(),
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    assert_eq!(graph.export_memory_json().unwrap(), before);
}
#[test]
fn future_assessments_do_not_change_historical_independence() {
    let (mut graph, claim, features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut GraphTierRegistry::new(),
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    let evidence = graph.evidence_store().clone();
    let stores = graph.epistemic_stores_mut();
    let old = TemporalTimestamp::new("2026-09-06T00:00:00Z").unwrap();
    let structure = stores
        .claims
        .assign_independence_clusters(
            &claim,
            &VerdictAsOf::new(old.clone(), old),
            &evidence,
            &stores.observations,
            &stores.sources,
        )
        .unwrap();
    assert_eq!(structure.supporting_cluster_count(), 3);
    assert!(structure.clusters().iter().all(|c| !c.has_evidence_risk()));
}

#[test]
fn dense_duplication_is_one_explained_group_not_quadratic_annotations() {
    let (mut graph, claim, mut features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    for i in 3..16 {
        graph
            .create_evidence(EvidenceInput::new(
                id(i),
                format!("source-{i}"),
                "alpha river crossed north",
            ))
            .unwrap();
        let claims = &mut graph.epistemic_stores_mut().claims;
        claims.register_evidence(id(i));
        claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Evidence(id(i)),
                claim.clone(),
                ClaimLinkKind::Supports,
            ))
            .unwrap();
        features.push(EvidenceRiskFeatures::new(id(i), "fixture-v1"));
    }
    let findings = graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut GraphTierRegistry::new(),
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].evidence_ids.len(), 15);
    assert_eq!(findings[0].witnesses.len(), 105);
    features.reverse();
    assert_eq!(
        detect_evidence_risks(&graph, &claim, &features).unwrap(),
        findings
    );
    for record in graph.evidence_store().records() {
        assert_eq!(
            record.risk_assessment_ids().len(),
            usize::from(record.id() != &id(2))
        );
    }
}

#[test]
fn large_existing_component_stores_quarantine_proof_once() {
    let (mut graph, claim, features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    for i in 3..103 {
        graph
            .create_evidence(EvidenceInput::new(
                id(i),
                "source-0",
                "existing contextual evidence",
            ))
            .unwrap();
        let claims = &mut graph.epistemic_stores_mut().claims;
        claims.register_evidence(id(i));
        claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Evidence(id(i)),
                claim.clone(),
                ClaimLinkKind::Supports,
            ))
            .unwrap();
    }
    let baseline_bytes = graph.export_memory_json().unwrap().len();
    let mut tiers = GraphTierRegistry::new();
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut tiers,
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    assert_eq!(
        tiers.tier_of(&TierRecordRef::Evidence(id(102))),
        GraphTier::Quarantine
    );
    assert!(
        graph.export_memory_json().unwrap().len() < baseline_bytes * 4,
        "quarantine history must not be copied into every component member"
    );
}

#[test]
fn corrupted_receipt_or_missing_reference_cannot_silently_drop_risk_on_restore() {
    let (mut graph, claim, features) = fixture(Some(EvidenceRiskSignal::LexicalDuplication));
    graph
        .apply_evidence_risks(
            &claim,
            &features,
            stamp(),
            "actor",
            &mut GraphTierRegistry::new(),
            &mut ImmuneResponder::new(),
        )
        .unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_str(&graph.export_memory_json().unwrap()).unwrap();
    let mut bad = snapshot.clone();
    bad["evidence"]["risk_assessments"][0]["assessment"]["finding"]["reason"] =
        "modified receipt".into();
    assert!(Graph::from_memory_json(&bad.to_string()).is_err());
    let mut missing = snapshot;
    missing["evidence"]["records"][0]["risk_assessment_ids"][0] = "missing-receipt".into();
    assert!(Graph::from_memory_json(&missing.to_string()).is_err());
}
