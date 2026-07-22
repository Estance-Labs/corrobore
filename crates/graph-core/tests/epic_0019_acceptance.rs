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
//! Epic 0019 acceptance suite: graph immune system.
//!
//! Exercises a deterministic seeded-attack corpus through structural,
//! epistemic, and behavioral detection, then through non-destructive response,
//! verification, and tier auditing at the public crate boundary.

use graph_core::{
    BehavioralBounds, BehavioralValidationInputs, BitemporalFactStore, BitemporalStamp, ClaimId,
    EpistemicValidationInputs, FactId, Graph, GraphTier, GraphTierRegistry, ImmuneResponder,
    ImmuneResponseAction, Node, NodeId, NodeInput, PheromoneDecay, PheromoneField,
    PheromoneTaskScope, ProbeAnswer, ProbeKind, ProbeRegistry, ProbeStatus, Relationship,
    RelationshipId, RelationshipInput, RelationshipType, RequestId, RetrievalTelemetryRecord,
    SkippedExpansionReason, TelemetryQueryDescriptor, TemporalTimestamp, TierRecordRef,
    TierTransitionReason, ValidationErrorRecord, WorkingSetDecisionEvent, WorkingSetId,
    WorkingSetTelemetryEvent, validate_graph_behavior, validate_graph_epistemics,
    validate_graph_structure,
};

const EXPECTED_FINDING_CODES: [&str; 10] = [
    "immune-structural--dangling-link",
    "immune-structural--impossible-cycle",
    "immune-structural--schema-violation",
    "immune-epistemic--unsupported-claim",
    "immune-epistemic--source-circularity",
    "immune-epistemic--open-contradiction",
    "immune-epistemic--stale-evidence",
    "immune-behavioral--pheromone-growth",
    "immune-behavioral--retrieval-drift",
    "immune-behavioral--centrality-shift",
];

/// Complete observable outcome of one deterministic seeded-attack run.
struct AcceptanceOutcome {
    findings: Vec<ValidationErrorRecord>,
    finding_codes: Vec<String>,
    nodes_before: Vec<Node>,
    nodes_after: Vec<Node>,
    relationships_before: Vec<Relationship>,
    relationships_after: Vec<Relationship>,
    tiers: GraphTierRegistry,
    responses: ImmuneResponder,
    probes: ProbeRegistry,
    probe_responses: usize,
    topological_poisoning_record: TierRecordRef,
    circular_source_record: TierRecordRef,
    repair_proposal_record: TierRecordRef,
    unvalidated_hypothesis_record: TierRecordRef,
}

/// Deterministic records injected into one Epic 0019 validation pass.
struct SeededAttackCorpus {
    graph: Graph,
    facts: BitemporalFactStore,
    as_of: TemporalTimestamp,
    evidence_facts: Vec<(RelationshipId, FactId)>,
    acyclic_relationship_types: Vec<RelationshipType>,
    pheromones: PheromoneField,
    scope: PheromoneTaskScope,
    audited_edges: Vec<RelationshipId>,
    telemetry: Vec<RetrievalTelemetryRecord>,
}

/// Build and run the Epic 0019 seeded-attack acceptance scenario.
fn run_seeded_attack_acceptance() -> AcceptanceOutcome {
    let corpus = seeded_attack_corpus();
    let nodes_before = corpus.graph.list_nodes().expect("nodes should list");
    let relationships_before = corpus
        .graph
        .list_relationships()
        .expect("relationships should list");

    let mut findings =
        validate_graph_structure(&corpus.graph, corpus.acyclic_relationship_types.as_slice())
            .expect("structural validation should run");
    findings.extend(
        validate_graph_epistemics(&EpistemicValidationInputs {
            graph: &corpus.graph,
            facts: &corpus.facts,
            as_of: &corpus.as_of,
            evidence_facts: corpus.evidence_facts.as_slice(),
            resolved_contradictions: &[],
        })
        .expect("epistemic validation should run"),
    );
    findings.extend(
        validate_graph_behavior(&BehavioralValidationInputs {
            pheromones: &corpus.pheromones,
            scope: &corpus.scope,
            edges: corpus.audited_edges.as_slice(),
            records: corpus.telemetry.as_slice(),
            bounds: &behavioral_bounds(),
        })
        .expect("behavioral validation should run"),
    );

    let topological_finding =
        finding_by_code(findings.as_slice(), "immune-behavioral--pheromone-growth");
    let circular_finding =
        finding_by_code(findings.as_slice(), "immune-epistemic--source-circularity");
    let schema_finding =
        finding_by_code(findings.as_slice(), "immune-structural--schema-violation");

    let mut tiers = GraphTierRegistry::new();
    let mut responses = ImmuneResponder::new();
    let topological_poisoning_record = tier_record(topological_finding);
    responses
        .quarantine(&mut tiers, topological_finding, "immune--epic-0019")
        .expect("topological poisoning should be quarantined");

    let circular_source_record = tier_record(circular_finding);
    let circular_transition = responses
        .quarantine(&mut tiers, circular_finding, "immune--epic-0019")
        .expect("circular-source pattern should be quarantined")
        .tier_transition_sequence
        .expect("quarantine should link its tier transition");

    let repair_proposal_record = TierRecordRef::Relationship(
        RelationshipId::new("relationship--epic-0019-repair-proposal")
            .expect("repair proposal ID should be valid"),
    );
    responses
        .propose_repair(
            &mut tiers,
            schema_finding,
            repair_proposal_record.clone(),
            "immune--epic-0019",
        )
        .expect("repair proposal should enter shadow");

    let unvalidated_hypothesis_record = TierRecordRef::Relationship(
        RelationshipId::new("relationship--epic-0019-hypothesis")
            .expect("hypothesis relationship ID should be valid"),
    );
    tiers
        .transition(
            unvalidated_hypothesis_record.clone(),
            GraphTier::Hypothesis,
            "immune--epic-0019",
            TierTransitionReason::ValidatorFinding,
        )
        .expect("unvalidated relation should enter hypothesis");

    let mut probes = ProbeRegistry::new();
    let probe_refs = probes.generate_from_findings(findings.as_slice());
    for probe_ref in &probe_refs {
        let probe = probes
            .probe(probe_ref)
            .expect("generated probe should be readable");
        let finding = findings
            .iter()
            .find(|finding| {
                finding.code() == probe.finding_code && finding.target() == &probe.target
            })
            .expect("each probe should retain its originating finding");
        responses.request_verification(finding, probe_ref.clone());
    }

    let circular_probe_ref = probe_refs
        .iter()
        .find(|probe_ref| {
            probes
                .probe(probe_ref)
                .is_some_and(|probe| probe.kind == ProbeKind::CircularDependency)
        })
        .expect("circularity finding should generate a probe")
        .clone();
    probes
        .answer(
            circular_probe_ref.as_str(),
            ProbeAnswer::Supported,
            Some(format!("transition--{circular_transition}")),
        )
        .expect("circularity probe should justify quarantine");

    let finding_codes = findings
        .iter()
        .map(|finding| finding.code().to_owned())
        .collect();
    let nodes_after = corpus.graph.list_nodes().expect("nodes should list");
    let relationships_after = corpus
        .graph
        .list_relationships()
        .expect("relationships should list");

    AcceptanceOutcome {
        findings,
        finding_codes,
        nodes_before,
        nodes_after,
        relationships_before,
        relationships_after,
        tiers,
        responses,
        probes,
        probe_responses: probe_refs.len(),
        topological_poisoning_record,
        circular_source_record,
        repair_proposal_record,
        unvalidated_hypothesis_record,
    }
}

/// Seed the complete defect corpus declared by issue #325.
fn seeded_attack_corpus() -> SeededAttackCorpus {
    let mut graph = Graph::new();

    let dangling_source = create_node(&mut graph, &["Campaign"]);
    let dangling_target = create_node(&mut graph, &["Narrative"]);
    create_relationship(&mut graph, &dangling_source, "PROMOTES", &dangling_target);
    graph
        .tombstone_node(&dangling_target)
        .expect("dangling target should be tombstoned");

    let cycle_parent = create_node(&mut graph, &["Campaign"]);
    let cycle_child = create_node(&mut graph, &["Narrative"]);
    create_relationship(&mut graph, &cycle_parent, "PART_OF", &cycle_child);
    create_relationship(&mut graph, &cycle_child, "PART_OF", &cycle_parent);

    let invalid_reporter = create_node(&mut graph, &["Post"]);
    let invalid_observation = create_node(&mut graph, &["Observation"]);
    create_relationship(
        &mut graph,
        &invalid_reporter,
        "REPORTS",
        &invalid_observation,
    );

    create_node(&mut graph, &["Claim"]);

    let circular_claim = create_node(&mut graph, &["Claim"]);
    let circular_source = create_node(&mut graph, &["Source"]);
    let first_observation = create_node(&mut graph, &["Observation"]);
    let second_observation = create_node(&mut graph, &["Observation"]);
    create_relationship(&mut graph, &circular_source, "REPORTS", &first_observation);
    create_relationship(&mut graph, &circular_source, "REPORTS", &second_observation);
    let stale_support =
        create_relationship(&mut graph, &first_observation, "SUPPORTS", &circular_claim);
    create_relationship(&mut graph, &second_observation, "SUPPORTS", &circular_claim);

    let contradicting_observation = create_node(&mut graph, &["Observation"]);
    create_relationship(
        &mut graph,
        &contradicting_observation,
        "CONTRADICTS",
        &circular_claim,
    );

    let poisoning_source = create_node(&mut graph, &["Campaign"]);
    let poisoning_hub = create_node(&mut graph, &["Narrative"]);
    let poisoning_edge =
        create_relationship(&mut graph, &poisoning_source, "AMPLIFIES", &poisoning_hub);

    let mut facts = BitemporalFactStore::new();
    let stale_fact = FactId::new("fact--epic-0019-stale").expect("stale fact ID should be valid");
    facts
        .assert_fact_state(
            stale_fact.clone(),
            "The seeded evidence was only valid during January",
            BitemporalStamp::new(
                timestamp("2026-01-01T00:00:00Z"),
                timestamp("2026-01-02T00:00:00Z"),
            )
            .expect("bitemporal stamp should be valid")
            .with_valid_to(timestamp("2026-02-01T00:00:00Z"))
            .expect("valid-to should follow valid-from"),
        )
        .expect("stale fact should be asserted");

    let telemetry = topological_poisoning_telemetry(&poisoning_edge, &poisoning_hub);
    let mut pheromones =
        PheromoneField::new(PheromoneDecay::new(1.0).expect("decay should be valid"));
    for record in &telemetry {
        pheromones.apply_retrieval_record(record);
    }

    SeededAttackCorpus {
        graph,
        facts,
        as_of: timestamp("2026-06-01T00:00:00Z"),
        evidence_facts: vec![(stale_support, stale_fact)],
        acyclic_relationship_types: vec![
            RelationshipType::new("PART_OF").expect("relationship type should be valid"),
        ],
        pheromones,
        scope: PheromoneTaskScope::task("fimi_investigation"),
        audited_edges: vec![poisoning_edge],
        telemetry,
    }
}

/// Encode poisoning as rapid edge growth, retrieval drift, and hub inflation.
fn topological_poisoning_telemetry(
    poisoning_edge: &RelationshipId,
    poisoning_hub: &NodeId,
) -> Vec<RetrievalTelemetryRecord> {
    vec![
        telemetry_record(
            "request--epic-0019-history-1",
            vec![
                expanded(poisoning_edge),
                selected(poisoning_hub),
                warm(
                    poisoning_hub,
                    "relationship--topology-warm-0",
                    "node--target-0",
                ),
            ],
        ),
        telemetry_record(
            "request--epic-0019-history-2",
            vec![expanded(poisoning_edge), selected(poisoning_hub)],
        ),
        telemetry_record(
            "request--epic-0019-poisoned",
            vec![
                expanded(poisoning_edge),
                selected(poisoning_hub),
                skipped(poisoning_hub, "relationship--topology-skip-1"),
                skipped(poisoning_hub, "relationship--topology-skip-2"),
                skipped(poisoning_hub, "relationship--topology-skip-3"),
                warm(
                    poisoning_hub,
                    "relationship--topology-warm-1",
                    "node--target-1",
                ),
                warm(
                    poisoning_hub,
                    "relationship--topology-warm-2",
                    "node--target-2",
                ),
                warm(
                    poisoning_hub,
                    "relationship--topology-warm-3",
                    "node--target-3",
                ),
                warm(
                    poisoning_hub,
                    "relationship--topology-warm-4",
                    "node--target-4",
                ),
            ],
        ),
    ]
}

fn telemetry_record(
    retrieval: &str,
    decisions: Vec<WorkingSetDecisionEvent>,
) -> RetrievalTelemetryRecord {
    RetrievalTelemetryRecord {
        retrieval_id: RequestId::new(retrieval).expect("retrieval ID should be valid"),
        working_set_id: WorkingSetId::new("working-set--epic-0019")
            .expect("working set ID should be valid"),
        descriptor: TelemetryQueryDescriptor {
            query_text: Some("detect topological poisoning".to_owned()),
            profile_kind: None,
            task_label: Some("fimi_investigation".to_owned()),
        },
        events: decisions
            .into_iter()
            .enumerate()
            .map(|(index, decision)| WorkingSetTelemetryEvent {
                sequence: index as u64,
                decision,
            })
            .collect(),
        outcome: None,
    }
}

fn expanded(relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeExpanded {
        relationship_id: relationship.clone(),
    }
}

fn selected(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::SeedSelected {
        node_id: node.clone(),
        marked_hot: true,
    }
}

fn skipped(source: &NodeId, relationship: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeSkipped {
        source_node_id: source.clone(),
        candidate_node_id: None,
        relationship_id: Some(
            RelationshipId::new(relationship).expect("skipped relationship ID should be valid"),
        ),
        reason: SkippedExpansionReason::BudgetLimit,
    }
}

fn warm(source: &NodeId, relationship: &str, target: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::WarmAdjacencyAttached {
        source_node_id: source.clone(),
        relationship_id: RelationshipId::new(relationship)
            .expect("warm relationship ID should be valid"),
        target_node_id: NodeId::new(target).expect("warm target ID should be valid"),
    }
}

fn behavioral_bounds() -> BehavioralBounds {
    BehavioralBounds {
        max_access_frequency: 2.0,
        max_drift_ratio: 0.5,
        max_degree_jump: 2,
    }
}

fn create_node(graph: &mut Graph, labels: &[&str]) -> NodeId {
    graph
        .create_node(NodeInput::new(labels.iter().copied()))
        .expect("seeded node should be created")
}

fn create_relationship(
    graph: &mut Graph,
    source: &NodeId,
    relationship_type: &str,
    target: &NodeId,
) -> RelationshipId {
    graph
        .create_relationship(
            RelationshipInput::new(source.clone(), relationship_type, target.clone())
                .expect("seeded relationship input should be valid"),
        )
        .expect("seeded relationship should be created")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp should be valid")
}

fn finding_by_code<'a>(
    findings: &'a [ValidationErrorRecord],
    code: &str,
) -> &'a ValidationErrorRecord {
    findings
        .iter()
        .find(|finding| finding.code() == code)
        .unwrap_or_else(|| panic!("expected finding {code}"))
}

fn tier_record(finding: &ValidationErrorRecord) -> TierRecordRef {
    match finding.target() {
        graph_core::ValidationTarget::Node(value) => {
            TierRecordRef::Node(NodeId::new(value).expect("finding node ID should be valid"))
        }
        graph_core::ValidationTarget::Relationship(value) => TierRecordRef::Relationship(
            RelationshipId::new(value).expect("finding relationship ID should be valid"),
        ),
        graph_core::ValidationTarget::Claim(value) => {
            TierRecordRef::Claim(ClaimId::new(value).expect("finding claim ID should be valid"))
        }
        target => panic!("finding target {target:?} is not tier-trackable"),
    }
}

//
// Acceptance: every defect seeded by the Epic 0019 corpus is detected by the
// matching validator, in deterministic order, without mutating the graph.
//
// Given two independently built copies of the seeded-attack corpus,
// when the three validator families run,
// then they should report the same complete finding set and preserve the
// canonical graph exactly.
#[test]
fn acceptance_seeded_attack_corpus_detects_every_defect_deterministically() {
    let first = run_seeded_attack_acceptance();
    let second = run_seeded_attack_acceptance();

    assert_eq!(first.finding_codes, EXPECTED_FINDING_CODES);
    assert_eq!(first.findings, second.findings);
    assert_eq!(first.nodes_before, first.nodes_after);
    assert_eq!(first.relationships_before, first.relationships_after);
}

//
// Acceptance: immune responses are non-destructive and fully auditable.
//
// Given the detected seeded attacks,
// when the response pipeline isolates, proposes repairs, and requests probes,
// then poisoning and circular sources should be quarantined, corrections
// should remain outside canonical, and every probe request should be linked
// through the response and tier audits.
#[test]
fn acceptance_responses_quarantine_attacks_without_rewriting_canonical() {
    let outcome = run_seeded_attack_acceptance();

    assert_eq!(
        outcome.tiers.tier_of(&outcome.topological_poisoning_record),
        GraphTier::Quarantine
    );
    assert_eq!(
        outcome.tiers.tier_of(&outcome.circular_source_record),
        GraphTier::Quarantine
    );
    assert_eq!(
        outcome.tiers.tier_of(&outcome.repair_proposal_record),
        GraphTier::Shadow
    );
    assert_eq!(
        outcome
            .tiers
            .tier_of(&outcome.unvalidated_hypothesis_record),
        GraphTier::Hypothesis
    );
    assert!(
        outcome
            .tiers
            .audit_trail()
            .iter()
            .all(|transition| transition.to != GraphTier::Canonical)
    );

    let probe_kinds: Vec<ProbeKind> = outcome
        .probes
        .probes()
        .iter()
        .map(|probe| probe.kind)
        .collect();
    assert_eq!(
        probe_kinds,
        vec![
            ProbeKind::StillSupported,
            ProbeKind::CircularDependency,
            ProbeKind::IndependentSource,
            ProbeKind::StillSupported,
        ]
    );
    assert_eq!(outcome.probes.probes().len(), outcome.probe_responses);
    assert_eq!(
        outcome
            .responses
            .audit()
            .iter()
            .filter(|response| response.tier_transition_sequence.is_some())
            .count(),
        3
    );
    assert_eq!(outcome.tiers.audit_trail().len(), 4);

    for response in outcome.responses.audit() {
        if let Some(sequence) = response.tier_transition_sequence {
            assert!(
                outcome
                    .tiers
                    .audit_trail()
                    .iter()
                    .any(|transition| transition.sequence == sequence),
                "every response tier link should resolve to an audited transition"
            );
        }
        if let ImmuneResponseAction::RequestVerification { probe_ref } = &response.action {
            let probe = outcome
                .probes
                .probe(probe_ref)
                .expect("every verification response should reference a probe");
            assert_eq!(probe.finding_code, response.finding_code);
            assert_eq!(probe.target, response.finding_target);
        }
    }

    let circular_transition = outcome
        .tiers
        .audit_for(&outcome.circular_source_record)
        .into_iter()
        .next()
        .expect("circular-source quarantine should be audited");
    let circular_probe = outcome
        .probes
        .probes()
        .iter()
        .find(|probe| probe.kind == ProbeKind::CircularDependency)
        .expect("circular-source finding should have a probe");
    assert_eq!(
        circular_probe.status,
        ProbeStatus::Answered(ProbeAnswer::Supported)
    );
    assert_eq!(
        circular_probe.justifies.as_deref(),
        Some(format!("transition--{}", circular_transition.sequence).as_str())
    );
    assert_eq!(outcome.probes.lifecycle().len(), 1);
}
