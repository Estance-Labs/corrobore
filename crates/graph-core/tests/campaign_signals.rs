#![allow(clippy::unwrap_used)]
//! Campaign signals are coordination evidence with dependency effects, never attribution.
use graph_core::*;

const NARRATIVE: &str = "narrative--relief-convoy";
const CAMPAIGN: &str = "campaign--relief-convoy";
const PROMPT_ARTIFACT: &str = "as an ai language model i cannot verify this";
const FINGERPRINT: &str = "style:fixture-cadence-v1";
const INFRASTRUCTURE: &str = "origin:shared-host";

fn stamp() -> BitemporalStamp {
    let time = TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap();
    BitemporalStamp::new(time.clone(), time).unwrap()
}
fn source(name: &str) -> SourceId {
    SourceId::new(format!("source--{name}")).unwrap()
}
fn evidence(name: &str) -> EvidenceId {
    EvidenceId::new(format!("evidence--{name}")).unwrap()
}
fn claim(name: &str) -> ClaimId {
    ClaimId::new(format!("claim--{name}")).unwrap()
}
fn narrative_scope() -> SignalScope {
    SignalScope::Narrative(NarrativeId::new(NARRATIVE).unwrap())
}
fn campaign_scope() -> SignalScope {
    SignalScope::Campaign(CampaignId::new(CAMPAIGN).unwrap())
}

// Two records support one claim so a dependency effect is observable on that
// claim, and a third supports another claim of the same collection so the
// collection scope covers what a single-claim assessment cannot see.
const RECORDS: [(&str, &str, &str); 3] = [
    ("a", "one", "alpha river crossed north"),
    ("b", "one", "beta mountain rain south"),
    ("c", "two", "gamma ocean calm west"),
];

struct Fixture {
    graph: Graph,
    features: Vec<CampaignSignalFeatures>,
}

// One fixture per signal: only the metadata that signal reads is shared, so a
// detection can only come from the mechanism under test.
fn fixture(signal: Option<CampaignSignal>) -> Fixture {
    let mut graph = Graph::new();
    let mut features = Vec::new();
    for name in ["one", "two"] {
        graph
            .epistemic_stores_mut()
            .claims
            .create_asserted_claim(ClaimInput::new(
                claim(name),
                ClaimStatement::new("one aid convoy was delayed at a checkpoint").unwrap(),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("convoy", None)),
            ))
            .unwrap();
    }
    for (name, target, text) in RECORDS {
        graph
            .epistemic_stores_mut()
            .sources
            .register_source(SourceInput::new(
                source(name),
                format!("https://example.test/{name}"),
                EvidenceSourceType::Document,
            ))
            .unwrap();
        let text = if signal == Some(CampaignSignal::CrossContentRedundancy) && name == "b" {
            "alpha river crossed north again"
        } else {
            text
        };
        graph
            .create_evidence(
                EvidenceInput::new(evidence(name), format!("https://example.test/{name}"), text)
                    .with_source_id(source(name)),
            )
            .unwrap();
        let claims = &mut graph.epistemic_stores_mut().claims;
        claims.register_evidence(evidence(name));
        claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Evidence(evidence(name)),
                    claim(target),
                    ClaimLinkKind::Supports,
                )
                .with_bitemporal(stamp()),
            )
            .unwrap();
        let mut feature = CampaignSignalFeatures::new(evidence(name), "coordination-review-v1");
        if name != "c" {
            match signal {
                Some(CampaignSignal::RepeatedPromptArtifact) => {
                    feature.prompt_artifacts = vec![PROMPT_ARTIFACT.into()];
                }
                Some(CampaignSignal::GenerationStyleFingerprint) => {
                    feature.generation_style_fingerprint = Some(FINGERPRINT.into());
                }
                Some(CampaignSignal::SharedInfrastructure) => {
                    feature.infrastructure = vec![INFRASTRUCTURE.into()];
                }
                _ => {}
            }
        }
        features.push(feature);
    }
    // Co-membership needs two declared content sources; every other fixture
    // declares one, so co-membership cannot fire beside the signal under test.
    let content = if signal == Some(CampaignSignal::NarrativeCoMembership) {
        vec![source("a"), source("b")]
    } else {
        vec![source("a")]
    };
    let membership = ContextMembership {
        claims: vec![claim("one"), claim("two")],
        themes: vec!["relief".into()],
        content,
        infrastructure: vec![],
        actors: vec![],
    };
    graph
        .create_narrative(NarrativeInput::new(
            NarrativeId::new(NARRATIVE).unwrap(),
            membership,
            stamp(),
        ))
        .unwrap();
    graph
        .create_campaign(CampaignInput::new(
            CampaignId::new(CAMPAIGN).unwrap(),
            vec![NarrativeId::new(NARRATIVE).unwrap()],
            ContextMembership::default(),
            stamp(),
        ))
        .unwrap();
    Fixture { graph, features }
}

fn clusters(graph: &Graph, target: &ClaimId) -> SourceIndependence {
    let evidence = graph.evidence_store().clone();
    let mut claims = graph.epistemic_stores().claims.clone();
    claims
        .assign_independence_clusters(
            target,
            &VerdictAsOf::new(stamp().valid_from, stamp().transaction_time),
            &evidence,
            &graph.epistemic_stores().observations,
            &graph.epistemic_stores().sources,
        )
        .unwrap()
}

//
// Each signal is detected on its own fixture, with the exact records, their
// distinct sources, and a reason carrying the measurement and its attribution.
#[test]
fn each_signal_has_an_isolated_fixture_and_explanation() {
    for signal in CampaignSignal::ALL {
        let fixture = fixture(Some(signal));
        let findings =
            detect_campaign_signals(&fixture.graph, &narrative_scope(), &fixture.features).unwrap();
        assert_eq!(
            findings.iter().map(CampaignSignalFinding::signal).collect::<Vec<_>>(),
            vec![signal],
            "{signal:?} fixture must detect exactly its own signal"
        );
        let finding = &findings[0];
        assert_eq!(finding.evidence_ids(), [evidence("a"), evidence("b")]);
        assert_eq!(finding.source_ids(), [source("a"), source("b")]);
        assert!(finding.reason().contains("ws-g-signal-v1"));
        assert!(finding.reason().contains("coordination-review-v1"));
        assert!(finding.group_id().starts_with("campaign-signal--"));
        assert_eq!(finding.scope(), &narrative_scope());
    }
}

//
// Metadata that is merely present is not coordination: nothing shared, nothing
// detected.
#[test]
fn benign_features_imply_no_campaign_signal() {
    let fixture = fixture(None);
    assert!(
        detect_campaign_signals(&fixture.graph, &narrative_scope(), &fixture.features)
            .unwrap()
            .is_empty()
    );
}

//
// The collection scope is the point: a fingerprint shared across two claims is
// invisible to a single-claim risk assessment and visible here.
#[test]
fn detection_spans_a_collection_where_a_single_claim_assessment_cannot() {
    let mut fixture = fixture(None);
    for name in ["b", "c"] {
        let index = RECORDS.iter().position(|(id, _, _)| *id == name).unwrap();
        fixture.features[index].generation_style_fingerprint = Some(FINGERPRINT.into());
    }
    let findings =
        detect_campaign_signals(&fixture.graph, &narrative_scope(), &fixture.features).unwrap();

    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].signal(),
        CampaignSignal::GenerationStyleFingerprint
    );
    assert_eq!(findings[0].evidence_ids(), [evidence("b"), evidence("c")]);
    for target in ["one", "two"] {
        let risk_features: Vec<_> = fixture
            .features
            .iter()
            .filter(|feature| {
                RECORDS.iter().any(|(name, claim_name, _)| {
                    *claim_name == target && evidence(name) == feature.evidence_id
                })
            })
            .map(|feature| {
                let mut risk =
                    EvidenceRiskFeatures::new(feature.evidence_id.clone(), "coordination-review-v1");
                risk.generation_fingerprint = feature.generation_style_fingerprint.clone();
                risk
            })
            .collect();
        assert!(
            detect_evidence_risks(&fixture.graph, &claim(target), &risk_features)
                .unwrap()
                .is_empty(),
            "a single-claim assessment cannot see a cross-claim pattern"
        );
    }
}

//
// A recorded signal joins the dependent links of one claim into one cluster and
// says why, so repetition through one production pipeline contributes once.
#[test]
fn recorded_signals_join_independence_clusters_with_an_explainable_reason() {
    let mut fixture = fixture(Some(CampaignSignal::GenerationStyleFingerprint));
    assert_eq!(
        clusters(&fixture.graph, &claim("one")).clusters().len(),
        2,
        "distinct sources start in distinct clusters"
    );

    let findings = fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();
    let structure = clusters(&fixture.graph, &claim("one"));

    assert_eq!(structure.clusters().len(), 1);
    let reason = structure.clusters()[0]
        .reasons()
        .iter()
        .find(|reason| reason.signal() == DependencySignal::CampaignSignal)
        .expect("the dependency must be explained as a campaign signal");
    assert!(reason.value().contains("generation_style_fingerprint"));
    assert!(reason.value().contains(findings[0].group_id()));
    assert_eq!(structure.supporting_cluster_count(), 1);
}

//
// Thematic grouping is curation, not shared production. Co-membership is kept as
// coordination context and must never collapse independence, or an analyst could
// deflate support by grouping content.
#[test]
fn narrative_co_membership_is_recorded_without_merging_independence_clusters() {
    let mut fixture = fixture(Some(CampaignSignal::NarrativeCoMembership));
    let findings = fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();

    assert_eq!(findings.len(), 1);
    assert!(!findings[0].affects_independence());
    assert!(!CampaignSignal::NarrativeCoMembership.affects_independence());
    let structure = clusters(&fixture.graph, &claim("one"));
    assert_eq!(structure.clusters().len(), 2);
    assert!(
        structure
            .clusters()
            .iter()
            .flat_map(IndependenceCluster::reasons)
            .all(|reason| reason.signal() != DependencySignal::CampaignSignal)
    );
}

//
// Recording coordination evidence is append-only and changes nothing factual:
// no claim, no verdict, no canonical node, and no second copy on replay.
#[test]
fn recording_signals_is_idempotent_and_changes_nothing_factual() {
    let mut fixture = fixture(Some(CampaignSignal::RepeatedPromptArtifact));
    let claims = fixture.graph.epistemic_stores().claims.clone();
    let verdicts = fixture.graph.epistemic_stores().verdicts.clone();

    fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();
    let after = fixture.graph.export_memory_json().unwrap();
    fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();

    assert_eq!(fixture.graph.export_memory_json().unwrap(), after);
    assert_eq!(fixture.graph.epistemic_stores().claims, claims);
    assert_eq!(fixture.graph.epistemic_stores().verdicts, verdicts);
    assert!(fixture.graph.list_nodes().unwrap().is_empty());
    assert_eq!(
        fixture
            .graph
            .evidence_store()
            .campaign_signals_for(&evidence("a"))
            .len(),
        1
    );
    assert!(
        fixture
            .graph
            .evidence_store()
            .campaign_signals_for(&evidence("c"))
            .is_empty()
    );
}

//
// A generation fingerprint says content shares a production pattern. It does not
// say who produced it, and no number of signals changes that.
#[test]
fn a_generation_fingerprint_alone_never_supports_attribution() {
    let mut fixture = fixture(Some(CampaignSignal::GenerationStyleFingerprint));
    let findings = fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();
    let request = AttributionRequest::new(
        NodeId::new("actor--suspected").unwrap(),
        narrative_scope(),
        findings.iter().map(|f| f.group_id().to_owned()).collect(),
        vec![],
    );

    assert_eq!(
        fixture
            .graph
            .assess_campaign_attribution(&request, &as_of())
            .unwrap(),
        AttributionAdmissibility::Refused(AttributionRefusal::CoordinationSignalsOnly)
    );
}

//
// Attribution needs justified confidence: a corroborating claim that the engine
// holds supported at the assessment point, not a signal and a hunch.
#[test]
fn attribution_requires_a_supported_corroborating_claim() {
    let mut fixture = fixture(Some(CampaignSignal::GenerationStyleFingerprint));
    let findings = fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();
    let cited: Vec<_> = findings.iter().map(|f| f.group_id().to_owned()).collect();
    let request = AttributionRequest::new(
        NodeId::new("actor--suspected").unwrap(),
        narrative_scope(),
        cited.clone(),
        vec![claim("one")],
    );

    assert_eq!(
        fixture
            .graph
            .assess_campaign_attribution(&request, &as_of())
            .unwrap(),
        AttributionAdmissibility::Refused(AttributionRefusal::CorroborationNotSupported(claim(
            "one"
        )))
    );

    support_claim(&mut fixture.graph, &claim("one"));

    assert_eq!(
        fixture
            .graph
            .assess_campaign_attribution(&request, &as_of())
            .unwrap(),
        AttributionAdmissibility::Admissible {
            corroborating: vec![claim("one")]
        }
    );
}

//
// An empty request and an unknown citation are refused with the reason, never
// silently treated as support.
#[test]
fn attribution_refuses_missing_and_unknown_support() {
    let fixture = fixture(Some(CampaignSignal::GenerationStyleFingerprint));
    let actor = NodeId::new("actor--suspected").unwrap();

    assert_eq!(
        fixture
            .graph
            .assess_campaign_attribution(
                &AttributionRequest::new(actor.clone(), narrative_scope(), vec![], vec![]),
                &as_of()
            )
            .unwrap(),
        AttributionAdmissibility::Refused(AttributionRefusal::NoSupportCited)
    );
    assert_eq!(
        fixture
            .graph
            .assess_campaign_attribution(
                &AttributionRequest::new(
                    actor,
                    narrative_scope(),
                    vec!["campaign-signal--absent".into()],
                    vec![]
                ),
                &as_of()
            )
            .unwrap(),
        AttributionAdmissibility::Refused(AttributionRefusal::UnknownSignal(
            "campaign-signal--absent".into()
        ))
    );
}

//
// A campaign scope covers the claims of the narratives it collects, so a
// campaign-level assessment sees what its narratives contain.
#[test]
fn a_campaign_scope_covers_the_claims_of_its_narratives() {
    let fixture = fixture(Some(CampaignSignal::SharedInfrastructure));
    let findings =
        detect_campaign_signals(&fixture.graph, &campaign_scope(), &fixture.features).unwrap();

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].signal(), CampaignSignal::SharedInfrastructure);
    assert_eq!(findings[0].scope(), &campaign_scope());
}

//
// Coordination evidence is durable: it survives a native round trip and keeps
// its dependency effect.
#[test]
fn recorded_signals_survive_a_native_round_trip() {
    let mut fixture = fixture(Some(CampaignSignal::GenerationStyleFingerprint));
    fixture
        .graph
        .record_campaign_signals(&narrative_scope(), &fixture.features, stamp())
        .unwrap();
    let exported = fixture.graph.export_memory_json().unwrap();
    let restored = Graph::from_memory_json(&exported).unwrap();

    assert_eq!(
        restored
            .evidence_store()
            .campaign_signals_for(&evidence("a"))
            .len(),
        1
    );
    assert_eq!(clusters(&restored, &claim("one")).clusters().len(), 1);
    assert_eq!(restored.export_memory_json().unwrap(), exported);
}

//
// A graph without coordination evidence exports exactly the bytes it did before
// the store existed.
#[test]
fn a_graph_without_campaign_signals_keeps_its_export_bytes() {
    let fixture = fixture(None);
    let exported = fixture.graph.export_memory_json().unwrap();

    assert!(!exported.contains("campaign_signals"));
}

//
// Detection refuses input it cannot ground: an unknown record, a record outside
// the collection, a blank attribution, and an oversized request.
#[test]
fn detection_refuses_ungrounded_or_unbounded_input() {
    let fixture = fixture(Some(CampaignSignal::GenerationStyleFingerprint));
    let mut unknown = fixture.features.clone();
    unknown.push(CampaignSignalFeatures::new(
        EvidenceId::new("evidence--absent").unwrap(),
        "coordination-review-v1",
    ));
    assert!(detect_campaign_signals(&fixture.graph, &narrative_scope(), &unknown).is_err());

    let mut blank = fixture.features.clone();
    blank[0].attribution = "  ".into();
    assert!(detect_campaign_signals(&fixture.graph, &narrative_scope(), &blank).is_err());

    let mut duplicated = fixture.features.clone();
    duplicated.push(fixture.features[0].clone());
    assert!(detect_campaign_signals(&fixture.graph, &narrative_scope(), &duplicated).is_err());

    assert!(
        detect_campaign_signals(
            &fixture.graph,
            &SignalScope::Narrative(NarrativeId::new("narrative--absent").unwrap()),
            &fixture.features
        )
        .is_err()
    );
}

fn as_of() -> VerdictAsOf {
    VerdictAsOf::new(stamp().valid_from, stamp().transaction_time)
}

// Give one claim an observation-bound supporting link and resolve it, so the
// corroboration gate has a genuinely supported claim to accept.
fn support_claim(graph: &mut Graph, target: &ClaimId) {
    let observation = ObservationId::new("observation--corroborating").unwrap();
    let evidence_store = graph.evidence_store().clone();
    let stores = graph.epistemic_stores_mut();
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source("a"),
                "the convoy was held four hours",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .unwrap();
    stores.claims.register_observation(observation.clone());
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                target.clone(),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(stamp()),
        )
        .unwrap();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence_store,
        &stores.observations,
        &stores.sources,
    );
    resolve_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        target,
        stamp(),
        "campaign-signal-test-v1",
    )
    .unwrap();
    assert_eq!(
        stores.verdicts.current_verdict(target).unwrap().state(),
        VerdictState::Supported
    );
}
