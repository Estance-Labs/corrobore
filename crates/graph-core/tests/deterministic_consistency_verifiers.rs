// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Contract tests for Epic 0029 WS-B item 3 (issue #165).

use graph_core::{
    ARITHMETIC_CONSISTENCY_VERIFIER_ID, ARITHMETIC_CONSISTENCY_VERIFIER_VERSION,
    ArithmeticConsistencyVerifier, BitemporalStamp, ClaimAnalyticalTarget,
    ClaimArithmeticConstraint, ClaimArithmeticPart, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimProposition, ClaimPropositionObject, ClaimStatement, ClaimStore,
    ClaimTarget, ClaimValidTimeScope, EvidenceId, EvidenceInput, EvidenceRecordStore,
    EvidenceSourceType, GRAPH_CONSISTENCY_VERIFIER_ID, GRAPH_CONSISTENCY_VERIFIER_VERSION, Graph,
    GraphConsistencyVerifier, NodeInput, ObservationId, ObservationInput, ObservationModality,
    ObservationStore, PropertyValue, RelationshipId, SCHEMA_CONSTRAINT_VERIFIER_ID,
    SCHEMA_CONSTRAINT_VERIFIER_VERSION, SchemaConstraintEvaluation, SchemaConstraintProvider,
    SchemaConstraintTarget, SchemaConstraintVerifier, SourceId, SourceInput, SourceStore,
    TEMPORAL_ORDERING_VERIFIER_ID, TEMPORAL_ORDERING_VERIFIER_VERSION, TemporalMetadata,
    TemporalOrderingVerifier, TemporalTimestamp, VerificationContext, VerificationRecord,
    VerificationRecordStore, VerificationResult, Verifier, VerifierCostClass, VerifierRegistry,
    VerifierSpec,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("claim id")
}

fn observation_id(value: &str) -> ObservationId {
    ObservationId::new(value).expect("observation id")
}

fn source_id() -> SourceId {
    SourceId::new("source--consistency-fixture").expect("source id")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp")
}

fn stamp() -> BitemporalStamp {
    BitemporalStamp::new(
        timestamp("2026-09-01T00:00:00Z"),
        timestamp("2026-09-06T12:00:00Z"),
    )
    .expect("stamp")
}

fn analytical_target() -> ClaimTarget {
    ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("consistency fixture", None))
}

#[derive(Default)]
struct EmptyVerificationStores {
    observations: ObservationStore,
    sources: SourceStore,
    evidence: EvidenceRecordStore,
}

fn create_claim(
    claims: &mut ClaimStore,
    id: &str,
    target: ClaimTarget,
    proposition: Option<ClaimProposition>,
    created_at: Option<&str>,
) {
    let mut input = ClaimInput::new(
        claim_id(id),
        ClaimStatement::new(format!("Fixture claim {id}")).expect("statement"),
        target,
    );
    if let Some(proposition) = proposition {
        input = input.with_proposition(proposition);
    }
    if let Some(created_at) = created_at {
        input = input.with_temporal(TemporalMetadata {
            created_at: Some(created_at.to_owned()),
            ..TemporalMetadata::default()
        });
    }
    claims.create_asserted_claim(input).expect("claim");
}

fn run(
    verifier: Box<dyn Verifier>,
    verifier_id: &str,
    version: &str,
    target_claim: &str,
    context: &VerificationContext<'_>,
) -> VerificationRecord {
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(verifier))
        .expect("registration");
    let mut records = VerificationRecordStore::new();
    let record_id = registry
        .run(
            verifier_id,
            version,
            &claim_id(target_claim),
            context,
            &mut records,
            stamp(),
        )
        .expect("verifier run");
    records.record_by_id(&record_id).expect("record").clone()
}

fn sources(acquired_at: Option<&str>) -> SourceStore {
    let mut sources = SourceStore::new();
    let mut input = SourceInput::new(
        source_id(),
        "https://evidence.example.org/records.json",
        EvidenceSourceType::Dataset,
    );
    if let Some(acquired_at) = acquired_at {
        input = input.with_acquired_at(timestamp(acquired_at));
    }
    sources.register_source(input).expect("source");
    sources
}

#[test]
fn temporal_ordering_passes_when_all_available_clocks_are_coherent() {
    let sources = sources(Some("2026-09-01T08:00:00Z"));
    let mut observations = ObservationStore::new();
    let observation = observation_id("observation--temporal-pass");
    observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source_id(),
                "payload",
                ObservationModality::Text,
            )
            .with_observed_at(timestamp("2026-09-01T09:00:00Z")),
            &sources,
        )
        .expect("observation");

    let proposition = ClaimProposition::new(
        "record--temporal",
        "active_count",
        ClaimPropositionObject::Literal(PropertyValue::Integer(2)),
    )
    .expect("proposition")
    .with_valid_time(
        ClaimValidTimeScope::new(
            Some(timestamp("2026-09-01T00:00:00Z")),
            Some(timestamp("2026-09-30T00:00:00Z")),
        )
        .expect("valid-time scope"),
    );
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--temporal-old",
        analytical_target(),
        Some(proposition),
        Some("2026-09-01T10:00:00Z"),
    );
    create_claim(
        &mut claims,
        "claim--temporal-new",
        analytical_target(),
        None,
        Some("2026-09-02T10:00:00Z"),
    );
    claims.register_observation(observation.clone());
    claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim_id("claim--temporal-old"),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(
                BitemporalStamp::new(
                    timestamp("2026-09-01T00:00:00Z"),
                    timestamp("2026-09-01T12:00:00Z"),
                )
                .expect("link stamp")
                .with_observation_time(timestamp("2026-09-01T09:00:00Z"))
                .with_publication_time(timestamp("2026-09-01T11:00:00Z")),
            ),
        )
        .expect("observation link");
    claims
        .attach_superseding_claim_to_claim(
            claim_id("claim--temporal-new"),
            claim_id("claim--temporal-old"),
            None,
        )
        .expect("superseding link");

    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(&claims, &observations, &sources, &empty.evidence);
    let record = run(
        Box::new(TemporalOrderingVerifier::new()),
        TEMPORAL_ORDERING_VERIFIER_ID,
        TEMPORAL_ORDERING_VERIFIER_VERSION,
        "claim--temporal-old",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Pass);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("valid-time")
            && text.contains("acquisition")
            && text.contains("supersession")
            && text.contains("bitemporal")
    }));
}

#[test]
fn temporal_ordering_fails_and_names_each_incoherent_clock() {
    let sources = sources(Some("2026-09-02T08:00:00Z"));
    let mut observations = ObservationStore::new();
    let observation = observation_id("observation--too-early");
    observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source_id(),
                "payload",
                ObservationModality::Text,
            )
            .with_observed_at(timestamp("2026-09-01T08:00:00Z")),
            &sources,
        )
        .expect("observation");

    let proposition: ClaimProposition = serde_json::from_value(serde_json::json!({
        "subject": "record--temporal",
        "predicate": "active_count",
        "object": {"Literal": {"Integer": 2}},
        "polarity": "Affirmed",
        "modality": "Asserted",
        "valid_time": {
            "valid_from": "2026-09-30T00:00:00Z",
            "valid_until": "2026-09-01T00:00:00Z"
        }
    }))
    .expect("persisted proposition fixture");
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--temporal-old",
        analytical_target(),
        Some(proposition),
        Some("2026-09-02T10:00:00Z"),
    );
    create_claim(
        &mut claims,
        "claim--temporal-new",
        analytical_target(),
        None,
        Some("2026-09-01T10:00:00Z"),
    );
    claims.register_observation(observation.clone());
    claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim_id("claim--temporal-old"),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(
                BitemporalStamp::new(
                    timestamp("2026-09-01T00:00:00Z"),
                    timestamp("2026-09-01T12:00:00Z"),
                )
                .expect("link stamp")
                .with_observation_time(timestamp("2026-09-01T11:30:00Z"))
                .with_publication_time(timestamp("2026-09-01T11:00:00Z")),
            ),
        )
        .expect("observation link");
    claims
        .attach_superseding_claim_to_claim(
            claim_id("claim--temporal-new"),
            claim_id("claim--temporal-old"),
            None,
        )
        .expect("superseding link");

    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(&claims, &observations, &sources, &empty.evidence);
    let record = run(
        Box::new(TemporalOrderingVerifier::new()),
        TEMPORAL_ORDERING_VERIFIER_ID,
        TEMPORAL_ORDERING_VERIFIER_VERSION,
        "claim--temporal-old",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("valid-time")
            && text.contains("observation--too-early")
            && text.contains("claim--temporal-new")
            && text.contains("publication_time")
    }));
}

#[test]
fn temporal_ordering_is_inconclusive_without_temporal_inputs() {
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--temporal-empty",
        analytical_target(),
        None,
        None,
    );
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    );
    let record = run(
        Box::new(TemporalOrderingVerifier::new()),
        TEMPORAL_ORDERING_VERIFIER_ID,
        TEMPORAL_ORDERING_VERIFIER_VERSION,
        "claim--temporal-empty",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Inconclusive);
}

fn arithmetic_claim(
    id: &str,
    value: PropertyValue,
    arithmetic: ClaimArithmeticConstraint,
) -> ClaimStore {
    let proposition = ClaimProposition::new(
        "aggregate--fixture",
        "total",
        ClaimPropositionObject::Literal(value),
    )
    .expect("proposition")
    .with_arithmetic_constraint(arithmetic);
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        id,
        analytical_target(),
        Some(proposition),
        None,
    );
    claims
}

#[test]
fn arithmetic_consistency_passes_for_bounded_unit_consistent_aggregate() {
    let claims = arithmetic_claim(
        "claim--arithmetic-pass",
        PropertyValue::Integer(100),
        ClaimArithmeticConstraint::new()
            .with_minimum(0.0)
            .with_maximum(100.0)
            .with_unit("items")
            .with_part(ClaimArithmeticPart::new(40.0).with_unit("items"))
            .with_part(ClaimArithmeticPart::new(60.0).with_unit("items")),
    );
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    );
    let record = run(
        Box::new(ArithmeticConsistencyVerifier::new()),
        ARITHMETIC_CONSISTENCY_VERIFIER_ID,
        ARITHMETIC_CONSISTENCY_VERIFIER_VERSION,
        "claim--arithmetic-pass",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Pass);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("bounds") && text.contains("unit") && text.contains("aggregate")
    }));
}

#[test]
fn arithmetic_consistency_fails_for_bounds_units_and_aggregate_drift() {
    let claims = arithmetic_claim(
        "claim--arithmetic-fail",
        PropertyValue::Float(120.0),
        ClaimArithmeticConstraint::new()
            .with_minimum(0.0)
            .with_maximum(100.0)
            .with_unit("items")
            .with_part(ClaimArithmeticPart::new(40.0).with_unit("items"))
            .with_part(ClaimArithmeticPart::new(60.0).with_unit("seconds")),
    );
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    );
    let record = run(
        Box::new(ArithmeticConsistencyVerifier::new()),
        ARITHMETIC_CONSISTENCY_VERIFIER_ID,
        ARITHMETIC_CONSISTENCY_VERIFIER_VERSION,
        "claim--arithmetic-fail",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("maximum") && text.contains("seconds") && text.contains("sum of parts")
    }));
}

#[test]
fn arithmetic_consistency_is_inconclusive_without_arithmetic_metadata() {
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--arithmetic-empty",
        analytical_target(),
        None,
        None,
    );
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    );
    let record = run(
        Box::new(ArithmeticConsistencyVerifier::new()),
        ARITHMETIC_CONSISTENCY_VERIFIER_ID,
        ARITHMETIC_CONSISTENCY_VERIFIER_VERSION,
        "claim--arithmetic-empty",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Inconclusive);
}

#[test]
fn graph_consistency_passes_for_resolved_target_and_current_links() {
    let mut graph = Graph::new();
    let node = graph.create_node(NodeInput::new(["Entity"])).expect("node");
    let evidence_id = EvidenceId::new("evidence--present").expect("evidence id");
    let mut evidence = EvidenceRecordStore::new();
    evidence
        .create_evidence(EvidenceInput::new(
            evidence_id.clone(),
            "source://fixture",
            "payload",
        ))
        .expect("evidence");
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--graph-pass",
        ClaimTarget::Node(node),
        None,
        None,
    );
    claims.register_evidence(evidence_id.clone());
    claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(evidence_id),
            claim_id("claim--graph-pass"),
            ClaimLinkKind::Supports,
        ))
        .expect("evidence link");
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(&claims, &empty.observations, &empty.sources, &evidence)
        .with_graph(&graph);
    let record = run(
        Box::new(GraphConsistencyVerifier::new()),
        GRAPH_CONSISTENCY_VERIFIER_ID,
        GRAPH_CONSISTENCY_VERIFIER_VERSION,
        "claim--graph-pass",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Pass);
}

#[test]
fn graph_consistency_fails_and_names_a_dangling_evidence_link() {
    let missing = EvidenceId::new("evidence--missing").expect("evidence id");
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--graph-fail",
        analytical_target(),
        None,
        None,
    );
    claims.register_evidence(missing.clone());
    claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(missing),
            claim_id("claim--graph-fail"),
            ClaimLinkKind::Supports,
        ))
        .expect("link admitted by claim store fixture");
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    );
    let record = run(
        Box::new(GraphConsistencyVerifier::new()),
        GRAPH_CONSISTENCY_VERIFIER_ID,
        GRAPH_CONSISTENCY_VERIFIER_VERSION,
        "claim--graph-fail",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(
        record
            .rationale()
            .is_some_and(|text| text.contains("evidence--missing") && text.contains("dangling"))
    );
}

#[test]
fn graph_consistency_fails_when_a_relationship_target_does_not_resolve() {
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--relationship-missing",
        ClaimTarget::Relationship(
            RelationshipId::new("relationship--missing").expect("relationship id"),
        ),
        None,
        None,
    );
    let graph = Graph::new();
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    )
    .with_graph(&graph);
    let record = run(
        Box::new(GraphConsistencyVerifier::new()),
        GRAPH_CONSISTENCY_VERIFIER_ID,
        GRAPH_CONSISTENCY_VERIFIER_VERSION,
        "claim--relationship-missing",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("relationship--missing") && text.contains("dangling")
    }));
}

#[test]
fn graph_consistency_fails_when_a_link_keeps_a_superseded_observation() {
    let sources = sources(None);
    let mut observations = ObservationStore::new();
    let old = observation_id("observation--old");
    let replacement = observation_id("observation--replacement");
    observations
        .create_observation(
            ObservationInput::new(old.clone(), source_id(), "old", ObservationModality::Text),
            &sources,
        )
        .expect("old observation");
    observations
        .supersede_observation(
            &old,
            ObservationInput::new(
                replacement.clone(),
                source_id(),
                "new",
                ObservationModality::Text,
            ),
            &sources,
        )
        .expect("replacement");
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--graph-superseded",
        analytical_target(),
        None,
        None,
    );
    claims.register_observation(old.clone());
    claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(old),
            claim_id("claim--graph-superseded"),
            ClaimLinkKind::Supports,
        ))
        .expect("old link");
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(&claims, &observations, &sources, &empty.evidence);
    let record = run(
        Box::new(GraphConsistencyVerifier::new()),
        GRAPH_CONSISTENCY_VERIFIER_ID,
        GRAPH_CONSISTENCY_VERIFIER_VERSION,
        "claim--graph-superseded",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("observation--old") && text.contains("observation--replacement")
    }));
}

#[test]
fn graph_consistency_is_inconclusive_without_graph_references_or_links() {
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--graph-empty",
        analytical_target(),
        None,
        None,
    );
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    );
    let record = run(
        Box::new(GraphConsistencyVerifier::new()),
        GRAPH_CONSISTENCY_VERIFIER_ID,
        GRAPH_CONSISTENCY_VERIFIER_VERSION,
        "claim--graph-empty",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Inconclusive);
}

#[derive(Debug)]
struct FixtureSchema {
    valid: bool,
}

impl SchemaConstraintProvider for FixtureSchema {
    fn evaluate(&self, target: SchemaConstraintTarget<'_>) -> SchemaConstraintEvaluation {
        match target {
            SchemaConstraintTarget::Node(node) if self.valid => {
                SchemaConstraintEvaluation::pass(format!("node:{}", node.id().as_str()))
            }
            SchemaConstraintTarget::Node(node) => SchemaConstraintEvaluation::fail(
                format!("node:{}", node.id().as_str()),
                ["required property 'name' is missing"],
            ),
            SchemaConstraintTarget::Relationship(_) => SchemaConstraintEvaluation::not_applicable(),
        }
    }
}

fn schema_fixture() -> (Graph, ClaimStore) {
    let mut graph = Graph::new();
    let node = graph.create_node(NodeInput::new(["Entity"])).expect("node");
    let mut claims = ClaimStore::new();
    create_claim(
        &mut claims,
        "claim--schema",
        ClaimTarget::Node(node),
        None,
        None,
    );
    (graph, claims)
}

#[test]
fn schema_constraint_passes_and_fails_from_an_installed_provider() {
    for (valid, expected) in [
        (true, VerificationResult::Pass),
        (false, VerificationResult::Fail),
    ] {
        let (graph, claims) = schema_fixture();
        let schema = FixtureSchema { valid };
        let empty = EmptyVerificationStores::default();
        let context = VerificationContext::new(
            &claims,
            &empty.observations,
            &empty.sources,
            &empty.evidence,
        )
        .with_graph(&graph)
        .with_schema_constraints(&schema);
        let record = run(
            Box::new(SchemaConstraintVerifier::new()),
            SCHEMA_CONSTRAINT_VERIFIER_ID,
            SCHEMA_CONSTRAINT_VERIFIER_VERSION,
            "claim--schema",
            &context,
        );

        assert_eq!(record.result(), expected);
        if !valid {
            assert!(
                record
                    .rationale()
                    .is_some_and(|text| text.contains("required property 'name'"))
            );
        }
    }
}

#[test]
fn schema_constraint_is_inconclusive_when_no_pack_is_installed() {
    let (graph, claims) = schema_fixture();
    let empty = EmptyVerificationStores::default();
    let context = VerificationContext::new(
        &claims,
        &empty.observations,
        &empty.sources,
        &empty.evidence,
    )
    .with_graph(&graph);
    let record = run(
        Box::new(SchemaConstraintVerifier::new()),
        SCHEMA_CONSTRAINT_VERIFIER_ID,
        SCHEMA_CONSTRAINT_VERIFIER_VERSION,
        "claim--schema",
        &context,
    );

    assert_eq!(record.result(), VerificationResult::Inconclusive);
    assert!(
        record
            .rationale()
            .is_some_and(|text| text.contains("no schema provider"))
    );
}

#[test]
fn consistency_verifier_ids_versions_and_costs_are_stable() {
    assert_eq!(TEMPORAL_ORDERING_VERIFIER_ID, "verifier.temporal-ordering");
    assert_eq!(TEMPORAL_ORDERING_VERIFIER_VERSION, "1.0.0");
    assert_eq!(
        ARITHMETIC_CONSISTENCY_VERIFIER_ID,
        "verifier.arithmetic-consistency"
    );
    assert_eq!(ARITHMETIC_CONSISTENCY_VERIFIER_VERSION, "1.0.0");
    assert_eq!(GRAPH_CONSISTENCY_VERIFIER_ID, "verifier.graph-consistency");
    assert_eq!(GRAPH_CONSISTENCY_VERIFIER_VERSION, "1.0.0");
    assert_eq!(SCHEMA_CONSTRAINT_VERIFIER_ID, "verifier.schema-constraint");
    assert_eq!(SCHEMA_CONSTRAINT_VERIFIER_VERSION, "1.0.0");

    let specs = [
        VerifierSpec::new(Box::new(TemporalOrderingVerifier::new())),
        VerifierSpec::new(Box::new(ArithmeticConsistencyVerifier::new())),
        VerifierSpec::new(Box::new(GraphConsistencyVerifier::new())),
        VerifierSpec::new(Box::new(SchemaConstraintVerifier::new())),
    ];
    assert_eq!(specs[0].cost_class(), VerifierCostClass::Medium);
    assert_eq!(specs[1].cost_class(), VerifierCostClass::Low);
    assert_eq!(specs[2].cost_class(), VerifierCostClass::Medium);
    assert_eq!(specs[3].cost_class(), VerifierCostClass::Medium);
    assert!(specs.iter().all(VerifierSpec::deterministic));
}
