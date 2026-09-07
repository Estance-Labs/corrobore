#![allow(clippy::unwrap_used)]
//! A supported claim is never permanently closed: routes say how it could be
//! re-checked, falsifiers say what would change it, and the publish gate
//! refuses a high-impact claim nobody could correct.
use graph_core::*;

const CLAIM: &str = "claim--reservoir-level";
const CLAIM_TYPE: &str = "measurement";
const MODEL_CHANNEL: &str = "model:extractor@3.1.0";

fn stamp() -> BitemporalStamp {
    let time = TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap();
    BitemporalStamp::new(time.clone(), time).unwrap()
}
fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).unwrap()
}
fn breakdown(benefit: f64) -> NextBestEvidenceScoreBreakdown {
    NextBestEvidenceScoreBreakdown::new(
        term(benefit),
        term(benefit),
        term(benefit),
        term(0.1),
        term(0.1),
        term(0.1),
    )
}
fn constraints() -> NextBestEvidenceConstraints {
    NextBestEvidenceConstraints::new(true, true, term(0.5))
}
fn route(
    id: &str,
    kind: CorrectiveRouteKind,
    channel: &str,
    liveness: RouteLiveness,
    benefit: f64,
) -> CorrectiveRouteInput {
    CorrectiveRouteInput::new(
        id,
        kind,
        CLAIM_TYPE,
        channel,
        liveness,
        breakdown(benefit),
        constraints(),
    )
    .unwrap()
}

// One claim produced by a named extraction pipeline, so a route on that same
// channel is a self-consistency recheck rather than a corrective one.
fn fixture() -> Graph {
    let mut graph = Graph::new();
    graph
        .create_evidence(
            EvidenceInput::new(
                EvidenceId::new("evidence--reading").unwrap(),
                "https://registry.test/reading",
                "the reservoir stood at 41 percent",
            )
            .with_extractor_id("extractor")
            .with_model_version("3.1.0"),
        )
        .unwrap();
    let stores = graph.epistemic_stores_mut();
    stores
        .claims
        .create_asserted_claim(
            ClaimInput::new(
                ClaimId::new(CLAIM).unwrap(),
                ClaimStatement::new("the reservoir stood at 41 percent on 3 May").unwrap(),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("measurement", None)),
            )
            .with_evidence_ref(EvidenceId::new("evidence--reading").unwrap()),
        )
        .unwrap();
    graph
}

fn falsifier(id: &str, routes: Vec<String>) -> FalsifierInput {
    FalsifierInput::new(
        id,
        ClaimId::new(CLAIM).unwrap(),
        "a registry reading above 60 percent for the same date",
        VerdictState::Refuted,
        routes,
        stamp(),
    )
    .unwrap()
}

fn actionable() -> ActionabilityAssessment {
    let mut dimensions = ConfidenceDimensions::default();
    dimensions.evidence_sufficiency = Some(Confidence::new(1.0).unwrap());
    dimensions.source_independence = Some(Confidence::new(1.0).unwrap());
    dimensions.contradiction_load = Some(Confidence::new(0.0).unwrap());
    dimensions.temporal_validity = Some(Confidence::new(1.0).unwrap());
    ActionabilityPolicy::default().evaluate(&dimensions, 2, true, VerdictState::Supported)
}

fn blocked() -> ActionabilityAssessment {
    ActionabilityPolicy::default().evaluate(
        &ConfidenceDimensions::default(),
        0,
        false,
        VerdictState::Unknown,
    )
}

//
// Every route kind is an existing investigation action, so a corrective route
// is a use of the Epic 0020 ranking rather than a second mechanism beside it.
#[test]
fn every_route_kind_maps_onto_an_existing_investigation_action() {
    assert_eq!(
        CorrectiveRouteKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>(),
        vec![
            "authoritative_api",
            "primary_document",
            "sensor",
            "signature_check",
            "independent_retrieval",
            "human_review",
        ]
    );
    for kind in CorrectiveRouteKind::ALL {
        assert!(InvestigationAction::ALL.contains(&kind.investigation_action()));
    }
    assert_eq!(
        CorrectiveRouteKind::HumanReview.investigation_action(),
        InvestigationAction::AskAnalyst
    );
    assert_eq!(
        CorrectiveRouteKind::SignatureCheck.investigation_action(),
        InvestigationAction::VerifyClaim
    );
}

//
// The catalogue is per claim type: a route for one claim type never answers for
// another, and identities stay unique and append-only.
#[test]
fn the_catalogue_serves_routes_per_claim_type() {
    let mut graph = fixture();
    let registry = route(
        "route--registry",
        CorrectiveRouteKind::AuthoritativeApi,
        "api:national-registry",
        RouteLiveness::Live,
        0.5,
    );
    graph.register_corrective_route(registry.clone()).unwrap();
    graph.register_corrective_route(registry).unwrap();
    graph
        .register_corrective_route(
            CorrectiveRouteInput::new(
                "route--other-type",
                CorrectiveRouteKind::PrimaryDocument,
                "archive:ministry",
                "channel:archive",
                RouteLiveness::Live,
                breakdown(0.5),
                constraints(),
            )
            .unwrap(),
        )
        .unwrap();

    let catalogue = &graph.epistemic_stores().corrective_routes;
    assert_eq!(catalogue.routes().len(), 2);
    assert_eq!(
        catalogue
            .routes_for_claim_type(CLAIM_TYPE)
            .iter()
            .map(|route| route.id())
            .collect::<Vec<_>>(),
        ["route--registry"]
    );
    assert!(catalogue.routes_for_claim_type("unknown").is_empty());
    assert!(catalogue.route_by_id("route--registry").is_some());

    let mut conflicting = graph.clone();
    assert!(
        conflicting
            .register_corrective_route(route(
                "route--registry",
                CorrectiveRouteKind::Sensor,
                "sensor:gauge",
                RouteLiveness::Live,
                0.5,
            ))
            .is_err(),
        "an identity cannot be reused for a different route"
    );
    for blank in ["", "   "] {
        assert!(
            CorrectiveRouteInput::new(
                blank,
                CorrectiveRouteKind::Sensor,
                CLAIM_TYPE,
                "sensor:gauge",
                RouteLiveness::Live,
                breakdown(0.5),
                constraints(),
            )
            .is_err()
        );
        assert!(
            CorrectiveRouteInput::new(
                "route--blank-channel",
                CorrectiveRouteKind::Sensor,
                CLAIM_TYPE,
                blank,
                RouteLiveness::Live,
                breakdown(0.5),
                constraints(),
            )
            .is_err()
        );
    }
}

//
// A falsifier states what would change a claim's state, names the routes that
// could produce it, and cannot claim that new evidence would confirm the claim:
// that is corroboration, not falsification.
#[test]
fn a_falsifier_records_what_would_change_a_supported_claim() {
    let mut graph = fixture();
    graph
        .register_corrective_route(route(
            "route--registry",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Live,
            0.5,
        ))
        .unwrap();
    let recorded = falsifier("falsifier--registry", vec!["route--registry".to_owned()]);
    graph.record_falsifier(recorded.clone()).unwrap();
    graph.record_falsifier(recorded).unwrap();

    let claim = ClaimId::new(CLAIM).unwrap();
    let records = graph.epistemic_stores().falsifiers.records_for_claim(&claim);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].would_change_to(), VerdictState::Refuted);
    assert_eq!(records[0].route_ids(), ["route--registry"]);
    assert!(records[0].expectation().contains("60 percent"));

    assert!(
        FalsifierInput::new(
            "falsifier--confirming",
            claim.clone(),
            "another confirming reading",
            VerdictState::Supported,
            vec!["route--registry".to_owned()],
            stamp(),
        )
        .is_err(),
        "a falsifier cannot promise to confirm the claim"
    );
    assert!(
        graph
            .record_falsifier(falsifier(
                "falsifier--unknown-route",
                vec!["route--absent".to_owned()],
            ))
            .is_err()
    );
    assert!(
        graph
            .record_falsifier(
                FalsifierInput::new(
                    "falsifier--unknown-claim",
                    ClaimId::new("claim--absent").unwrap(),
                    "a contradicting reading",
                    VerdictState::Refuted,
                    vec!["route--registry".to_owned()],
                    stamp(),
                )
                .unwrap()
            )
            .is_err()
    );
}

//
// An independent channel outranks a same-pipeline recheck whatever its expected
// value: asking the model that produced the claim to agree with itself is not a
// correction, so no score can promote it above an independent channel.
#[test]
fn an_independent_channel_always_outranks_a_self_consistent_recheck() {
    let mut graph = fixture();
    graph
        .register_corrective_route(route(
            "route--same-model",
            CorrectiveRouteKind::IndependentRetrieval,
            MODEL_CHANNEL,
            RouteLiveness::Live,
            1.0,
        ))
        .unwrap();
    graph
        .register_corrective_route(route(
            "route--registry",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Live,
            0.2,
        ))
        .unwrap();

    let claim = ClaimId::new(CLAIM).unwrap();
    assert_eq!(graph.producing_channels(&claim).unwrap(), [MODEL_CHANNEL]);
    let ranking = graph.corrective_route_ranking(&claim, CLAIM_TYPE).unwrap();

    assert_eq!(
        ranking
            .routes()
            .iter()
            .map(|route| route.route_id())
            .collect::<Vec<_>>(),
        ["route--registry", "route--same-model"]
    );
    assert_eq!(
        ranking.routes()[0].independence(),
        RouteIndependence::Independent
    );
    assert_eq!(
        ranking.routes()[1].independence(),
        RouteIndependence::SelfConsistent
    );
    assert_eq!(ranking.selected().unwrap().route_id(), "route--registry");
    assert_eq!(
        ranking.selected_independent().unwrap().route_id(),
        "route--registry"
    );
}

//
// Availability reuses the existing constraint vocabulary instead of a second
// eligibility policy, and a retired or unavailable route never counts as live.
#[test]
fn route_availability_reuses_the_next_best_evidence_constraints() {
    let mut graph = fixture();
    graph
        .register_corrective_route(
            CorrectiveRouteInput::new(
                "route--over-budget",
                CorrectiveRouteKind::Sensor,
                CLAIM_TYPE,
                "sensor:gauge",
                RouteLiveness::Live,
                breakdown(0.9),
                NextBestEvidenceConstraints::new(false, true, term(0.5)),
            )
            .unwrap(),
        )
        .unwrap();
    graph
        .register_corrective_route(
            CorrectiveRouteInput::new(
                "route--denied",
                CorrectiveRouteKind::PrimaryDocument,
                CLAIM_TYPE,
                "archive:ministry",
                RouteLiveness::Live,
                breakdown(0.9),
                NextBestEvidenceConstraints::new(true, false, term(0.5)),
            )
            .unwrap(),
        )
        .unwrap();
    graph
        .register_corrective_route(
            CorrectiveRouteInput::new(
                "route--risky",
                CorrectiveRouteKind::IndependentRetrieval,
                CLAIM_TYPE,
                "search:open-web",
                RouteLiveness::Live,
                NextBestEvidenceScoreBreakdown::new(
                    term(0.9),
                    term(0.9),
                    term(0.9),
                    term(0.1),
                    term(0.1),
                    term(0.9),
                ),
                constraints(),
            )
            .unwrap(),
        )
        .unwrap();
    graph
        .register_corrective_route(route(
            "route--retired",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:retired-registry",
            RouteLiveness::Retired,
            0.9,
        ))
        .unwrap();

    let ranking = graph
        .corrective_route_ranking(&ClaimId::new(CLAIM).unwrap(), CLAIM_TYPE)
        .unwrap();

    assert_eq!(ranking.routes().len(), 4);
    assert!(ranking.routes().iter().all(|route| !route.is_available()));
    assert!(ranking.selected().is_none());
    let reasons = |id: &str| {
        ranking
            .routes()
            .iter()
            .find(|route| route.route_id() == id)
            .unwrap()
            .ineligibility_reasons()
            .to_vec()
    };
    assert_eq!(
        reasons("route--over-budget"),
        [NextBestEvidenceIneligibilityReason::BudgetExceeded]
    );
    assert_eq!(
        reasons("route--denied"),
        [NextBestEvidenceIneligibilityReason::PolicyDenied]
    );
    assert!(matches!(
        reasons("route--risky").as_slice(),
        [NextBestEvidenceIneligibilityReason::SourceRiskExceeded { .. }]
    ));
    assert!(reasons("route--retired").is_empty(), "liveness is not an eligibility reason");
    assert!(!RouteLiveness::Retired.is_live());
    assert!(!RouteLiveness::Unavailable.is_live());
}

//
// A high-impact claim nobody could correct cannot be published, and each
// missing precondition is named separately.
#[test]
fn a_high_impact_claim_without_a_live_route_cannot_publish() {
    let claim = ClaimId::new(CLAIM).unwrap();
    let mut graph = fixture();
    let decision = graph
        .evaluate_publish_gate(&claim, ClaimImpact::High, CLAIM_TYPE, &actionable())
        .unwrap();
    assert!(!decision.may_publish());
    assert!(decision.blockers().contains(&PublishBlocker::CorrectiveRouteMissing));
    assert!(decision.blockers().contains(&PublishBlocker::FalsifierMissing));

    graph
        .register_corrective_route(route(
            "route--unavailable",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Unavailable,
            0.5,
        ))
        .unwrap();
    graph
        .record_falsifier(falsifier(
            "falsifier--registry",
            vec!["route--unavailable".to_owned()],
        ))
        .unwrap();
    let decision = graph
        .evaluate_publish_gate(&claim, ClaimImpact::High, CLAIM_TYPE, &actionable())
        .unwrap();
    assert_eq!(decision.blockers(), [PublishBlocker::CorrectiveRouteNotLive]);

    let mut self_consistent = graph.clone();
    self_consistent
        .register_corrective_route(route(
            "route--same-model",
            CorrectiveRouteKind::IndependentRetrieval,
            MODEL_CHANNEL,
            RouteLiveness::Live,
            0.9,
        ))
        .unwrap();
    let decision = self_consistent
        .evaluate_publish_gate(&claim, ClaimImpact::High, CLAIM_TYPE, &actionable())
        .unwrap();
    assert_eq!(
        decision.blockers(),
        [PublishBlocker::SelfConsistentRoutesOnly],
        "a same-pipeline recheck is not a corrective route for a high-impact claim"
    );

    graph
        .register_corrective_route(route(
            "route--registry",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Live,
            0.4,
        ))
        .unwrap();
    let decision = graph
        .evaluate_publish_gate(&claim, ClaimImpact::High, CLAIM_TYPE, &actionable())
        .unwrap();
    assert!(decision.may_publish());
    assert_eq!(decision.selected_route(), Some("route--registry"));
    assert_eq!(decision.falsifiers(), ["falsifier--registry"]);
}

//
// The gate composes the WS-D decision instead of restating it: it adds no
// dimension-level reason of its own and keeps the actionability blockers
// visible beside its own.
#[test]
fn the_publish_gate_composes_the_actionability_decision_without_repeating_it() {
    let claim = ClaimId::new(CLAIM).unwrap();
    let mut graph = fixture();
    graph
        .register_corrective_route(route(
            "route--registry",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Live,
            0.5,
        ))
        .unwrap();
    graph
        .record_falsifier(falsifier(
            "falsifier--registry",
            vec!["route--registry".to_owned()],
        ))
        .unwrap();

    let blocked_decision = graph
        .evaluate_publish_gate(&claim, ClaimImpact::High, CLAIM_TYPE, &blocked())
        .unwrap();
    assert!(!blocked_decision.may_publish());
    assert_eq!(blocked_decision.blockers(), [PublishBlocker::NotActionable]);
    assert!(
        blocked_decision
            .actionability_blockers()
            .contains(&ActionabilityBlocker::DeterministicVerificationMissing)
    );

    // A standard-impact claim needs no corrective route: the requirement is the
    // high-impact gate, not a new universal precondition.
    let standard = fixture();
    let decision = standard
        .evaluate_publish_gate(&claim, ClaimImpact::Standard, CLAIM_TYPE, &actionable())
        .unwrap();
    assert!(decision.may_publish());
    assert!(decision.blockers().is_empty());
    assert_eq!(decision.selected_route(), None);
}

//
// The claim audit answers what could make Corrobore change its mind, and says
// so explicitly when nothing could.
#[test]
fn the_claim_audit_answers_what_would_change_its_mind() {
    let claim = ClaimId::new(CLAIM).unwrap();
    let mut graph = fixture();
    let bare = graph.claim_audit_path(&claim).unwrap();
    assert!(
        bare["unverified_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "no_recorded_falsifier"),
        "a claim nobody could correct is an explicit gap"
    );
    assert!(bare["falsifiers"].as_array().unwrap().is_empty());

    graph
        .register_corrective_route(route(
            "route--registry",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Live,
            0.5,
        ))
        .unwrap();
    graph
        .record_falsifier(falsifier(
            "falsifier--registry",
            vec!["route--registry".to_owned()],
        ))
        .unwrap();
    let audit = graph.claim_audit_path(&claim).unwrap();

    assert_eq!(audit["falsifiers"][0]["id"], "falsifier--registry");
    assert_eq!(audit["falsifiers"][0]["would_change_to"], "Refuted");
    assert_eq!(audit["corrective_routes"][0]["id"], "route--registry");
    assert_eq!(audit["corrective_routes"][0]["kind"], "authoritative_api");
    assert!(
        !audit["unverified_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["kind"] == "no_recorded_falsifier")
    );
}

//
// Routes and falsifiers are governed records: they survive a native round trip,
// and a graph without them exports the bytes it did before.
#[test]
fn routes_and_falsifiers_survive_a_native_round_trip() {
    let claim = ClaimId::new(CLAIM).unwrap();
    let bare = fixture().export_memory_json().unwrap();
    assert!(!bare.contains("corrective_routes"));
    assert!(!bare.contains("falsifiers"));

    let mut graph = fixture();
    graph
        .register_corrective_route(route(
            "route--registry",
            CorrectiveRouteKind::AuthoritativeApi,
            "api:national-registry",
            RouteLiveness::Live,
            0.5,
        ))
        .unwrap();
    graph
        .record_falsifier(falsifier(
            "falsifier--registry",
            vec!["route--registry".to_owned()],
        ))
        .unwrap();
    let exported = graph.export_memory_json().unwrap();
    let restored = Graph::from_memory_json(&exported).unwrap();

    assert_eq!(restored.export_memory_json().unwrap(), exported);
    assert_eq!(
        restored
            .epistemic_stores()
            .falsifiers
            .records_for_claim(&claim)
            .len(),
        1
    );
    assert_eq!(
        restored.epistemic_stores().corrective_routes.routes().len(),
        1
    );
}
