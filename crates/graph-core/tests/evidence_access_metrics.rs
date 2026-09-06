#![allow(clippy::unwrap_used)]
use graph_core::*;
fn eid(s: &str) -> EvidenceId {
    EvidenceId::new(s).unwrap()
}
fn fixture() -> (Graph, VerificationInputs, VerdictAsOf) {
    let mut graph = Graph::new();
    let id = ClaimId::new("claim").unwrap();
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            id.clone(),
            ClaimStatement::new("A happened").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("A", None)),
        ))
        .unwrap();
    for name in ["linked", "unreachable", "residual"] {
        graph
            .create_evidence(EvidenceInput::new(eid(name), "source", name))
            .unwrap();
        graph
            .epistemic_stores_mut()
            .claims
            .register_evidence(eid(name));
    }
    let link = graph
        .epistemic_stores_mut()
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(eid("linked")),
            id.clone(),
            ClaimLinkKind::Supports,
        ))
        .unwrap();
    graph
        .epistemic_stores_mut()
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(eid("unreachable")),
            id.clone(),
            ClaimLinkKind::Contradicts,
        ))
        .unwrap();
    let time = TemporalTimestamp::new("2026-09-06T00:00:00Z").unwrap();
    (
        graph,
        VerificationInputs::for_claim(id).with_link_ref(link.reference_key()),
        VerdictAsOf::new(time.clone(), time),
    )
}
#[test]
fn present_but_not_on_examined_path_is_a_retrieval_failure() {
    let (graph, trace, at) = fixture();
    let report = measure_evidence_access(
        &graph,
        &trace,
        &at,
        &[eid("linked"), eid("unreachable"), eid("absent")],
        &[eid("linked"), eid("residual")],
    )
    .unwrap();
    assert_eq!(report.presence_rate, Some(2.0 / 3.0));
    assert_eq!(report.reachability_rate, Some(1.0 / 3.0));
    assert_eq!(report.present_but_unreachable, vec![eid("unreachable")]);
    assert_eq!(report.absent, vec![eid("absent")]);
    assert_eq!(report.residual_evidence, vec![eid("residual")]);
    assert_eq!(report.residual_evidence_rate, Some(0.5));
    assert!(
        report
            .proposals
            .iter()
            .any(|p| p.action == InvestigationAction::ExpandRelation)
    );
    assert!(
        report
            .proposals
            .iter()
            .any(|p| p.action == InvestigationAction::SearchCorpus)
    );
}
#[test]
fn retrieved_contradiction_is_explained_even_if_not_examined() {
    let (graph, trace, at) = fixture();
    let report = measure_evidence_access(
        &graph,
        &trace,
        &at,
        &[eid("unreachable")],
        &[eid("unreachable")],
    )
    .unwrap();
    assert_eq!(report.reachability_rate, Some(0.0));
    assert_eq!(report.residual_evidence_rate, Some(0.0));
    assert!(report.residual_evidence.is_empty());
}
#[test]
fn missing_denominators_are_unmeasured_and_bad_traces_rejected() {
    let (graph, trace, at) = fixture();
    let report = measure_evidence_access(&graph, &trace, &at, &[], &[]).unwrap();
    assert_eq!(report.presence_rate, None);
    assert_eq!(report.reachability_rate, None);
    assert_eq!(report.residual_evidence_rate, None);
    assert!(report.proposals.is_empty());
    assert!(measure_evidence_access(&graph, &trace, &at, &[], &[eid("absent")]).is_err());
    assert!(
        measure_evidence_access(
            &graph,
            &trace.clone().with_link_ref("unknown"),
            &at,
            &[],
            &[]
        )
        .is_err()
    );
}
#[test]
fn duplicate_ids_do_not_inflate_rates_and_graph_is_read_only() {
    let (graph, trace, at) = fixture();
    let before = graph.export_memory_json().unwrap();
    let report = measure_evidence_access(
        &graph,
        &trace,
        &at,
        &[eid("linked"), eid("linked")],
        &[eid("linked"), eid("linked")],
    )
    .unwrap();
    assert_eq!(report.presence_rate, Some(1.0));
    assert_eq!(report.reachability_rate, Some(1.0));
    assert_eq!(report.residual_evidence_rate, Some(0.0));
    assert_eq!(graph.export_memory_json().unwrap(), before);
}

#[test]
fn unexamined_bridge_cannot_make_another_claims_evidence_reachable() {
    let (mut graph, trace, at) = fixture();
    let other = ClaimId::new("other").unwrap();
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            other.clone(),
            ClaimStatement::new("B happened").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("B", None)),
        ))
        .unwrap();
    let evidence_link = graph
        .epistemic_stores_mut()
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(eid("residual")),
            other.clone(),
            ClaimLinkKind::Supports,
        ))
        .unwrap();
    let bridge = graph
        .epistemic_stores_mut()
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Claim(other),
            trace.claim_id().clone(),
            ClaimLinkKind::Supports,
        ))
        .unwrap();
    let incomplete = trace.with_link_ref(evidence_link.reference_key());
    assert!(
        measure_evidence_access(
            &graph,
            &incomplete,
            &at,
            &[eid("residual")],
            &[eid("residual")]
        )
        .is_err()
    );
    let complete = incomplete.with_link_ref(bridge.reference_key());
    let report = measure_evidence_access(
        &graph,
        &complete,
        &at,
        &[eid("residual")],
        &[eid("residual")],
    )
    .unwrap();
    assert_eq!(report.reachability_rate, Some(1.0));
    assert_eq!(report.residual_evidence_rate, Some(0.0));
}
#[test]
fn benchmark_fixture_preserves_distinct_series_and_actionable_residuals() {
    let (graph, trace, at) = fixture();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/evidence-access-v1.json")).unwrap();
    let ids = |key: &str| {
        fixture[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| eid(id.as_str().unwrap()))
            .collect::<Vec<_>>()
    };
    let report =
        measure_evidence_access(&graph, &trace, &at, &ids("expected"), &ids("retrieved")).unwrap();
    let actual = serde_json::to_value(&report).unwrap();
    for (key, value) in fixture["report"].as_object().unwrap() {
        assert_eq!(&actual[key], value, "{key}");
    }
    assert_eq!(
        actual["verification_inputs"],
        serde_json::to_value(&trace).unwrap()
    );
    assert_eq!(actual["as_of"], serde_json::to_value(&at).unwrap());
}

#[test]
fn residual_signal_can_select_retrieval_but_cannot_bypass_budget() {
    let (graph, trace, at) = fixture();
    let report = measure_evidence_access(&graph, &trace, &at, &[], &[eid("residual")]).unwrap();
    let zero = NextBestEvidenceScoreTerm::new(0.0).unwrap();
    let one = NextBestEvidenceScoreTerm::new(1.0).unwrap();
    let score = NextBestEvidenceScoreBreakdown::new(one, one, one, zero, zero, zero);
    let candidate = |budget| {
        report.proposals[0]
            .candidate(
                "residual-retrieval",
                score,
                NextBestEvidenceConstraints::new(budget, true, one),
            )
            .unwrap()
    };
    let allowed = rank_next_best_evidence(vec![candidate(true)]).unwrap();
    assert_eq!(
        allowed.selected().unwrap().action(),
        InvestigationAction::SearchCorpus
    );
    let denied = rank_next_best_evidence(vec![candidate(false)]).unwrap();
    assert!(denied.selected().is_none());
}

#[test]
fn future_links_do_not_explain_residuals_or_validate_an_examined_path() {
    let (mut graph, trace, at) = fixture();
    let future = TemporalTimestamp::new("2027-01-01T00:00:00Z").unwrap();
    let link = graph
        .epistemic_stores_mut()
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Evidence(eid("residual")),
                trace.claim_id().clone(),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(BitemporalStamp::new(future.clone(), future).unwrap()),
        )
        .unwrap();
    let report = measure_evidence_access(&graph, &trace, &at, &[], &[eid("residual")]).unwrap();
    assert_eq!(report.residual_evidence_rate, Some(1.0));
    assert!(
        measure_evidence_access(
            &graph,
            &trace.with_link_ref(link.reference_key()),
            &at,
            &[],
            &[eid("residual")]
        )
        .is_err()
    );
}

#[test]
fn explicitly_examined_observation_is_a_recorded_path_to_its_evidence() {
    let (mut graph, trace, at) = fixture();
    let source = SourceId::new("source-observed").unwrap();
    let observation = ObservationId::new("observation").unwrap();
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "urn:source:observed",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source.clone(),
                "observed fact",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .unwrap();
    stores.claims.register_observation(observation.clone());
    let link = stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(observation.clone()),
            trace.claim_id().clone(),
            ClaimLinkKind::ContextFor,
        ))
        .unwrap();
    graph
        .create_evidence(
            EvidenceInput::new(eid("observed"), "urn:source:observed", "observed fact")
                .with_source_id(source)
                .with_observation_id(observation.clone()),
        )
        .unwrap();
    let alone = trace.with_observation(observation);
    let before =
        measure_evidence_access(&graph, &alone, &at, &[eid("observed")], &[eid("observed")])
            .unwrap();
    assert_eq!(before.reachability_rate, Some(1.0));
    assert_eq!(before.residual_evidence_rate, Some(0.0));
    let after = measure_evidence_access(
        &graph,
        &alone.with_link_ref(link.reference_key()),
        &at,
        &[eid("observed")],
        &[eid("observed")],
    )
    .unwrap();
    assert_eq!(after.reachability_rate, Some(1.0));
}
