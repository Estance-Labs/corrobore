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
use graph_core::{
    BitemporalFactStore, BitemporalStamp, EpistemicValidationInputs, FactId, Graph, NodeId,
    NodeInput, RelationshipId, RelationshipInput, TemporalTimestamp, ValidationErrorSeverity,
    ValidationTarget, validate_graph_epistemics,
};

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("epistemic timestamp should be valid")
}

fn fact_id(value: &str) -> FactId {
    FactId::new(value).expect("epistemic fact ID should be valid")
}

fn create_node(graph: &mut Graph, labels: &[&str]) -> NodeId {
    graph
        .create_node(NodeInput::new(labels.iter().copied()))
        .expect("epistemic node should be created")
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
                .expect("epistemic relationship input should be valid"),
        )
        .expect("epistemic relationship should be created")
}

/// A well-formed scenario: one claim supported by two observations reported by
/// two independent sources.
fn independent_support_scenario() -> (Graph, NodeId) {
    let mut graph = Graph::new();
    let claim = create_node(&mut graph, &["Claim"]);
    for index in 0..2 {
        let source = create_node(&mut graph, &["Source"]);
        let observation = create_node(&mut graph, &["Observation"]);
        create_relationship(&mut graph, &source, "REPORTS", &observation);
        create_relationship(&mut graph, &observation, "SUPPORTS", &claim);
        let _ = index;
    }
    (graph, claim)
}

fn empty_inputs<'a>(
    graph: &'a Graph,
    facts: &'a BitemporalFactStore,
    as_of: &'a TemporalTimestamp,
) -> EpistemicValidationInputs<'a> {
    EpistemicValidationInputs {
        graph,
        facts,
        as_of,
        evidence_facts: &[],
        resolved_contradictions: &[],
    }
}

//
// Verify that a well-formed epistemic scenario produces no findings: two
// independent sources support the claim and nothing contradicts it.
//
// Given the independent-support scenario,
// when epistemic validation runs,
// then no findings should be reported.
#[test]
fn well_formed_scenarios_produce_no_findings() {
    let (graph, _claim) = independent_support_scenario();
    let facts = BitemporalFactStore::new();
    let as_of = ts("2026-06-01T00:00:00Z");

    let findings = validate_graph_epistemics(&empty_inputs(&graph, &facts, &as_of))
        .expect("epistemic validation should run");

    assert!(findings.is_empty());
}

//
// Verify unsupported-claim detection: a claim without any supporting
// observation or evidence is a typed epistemic finding, and support from a
// non-epistemic node does not count.
//
// Given one supported claim and one claim whose only support comes from a
// plain Post node,
// when epistemic validation runs,
// then exactly the unsupported claim should be flagged.
#[test]
fn unsupported_claims_are_detected() {
    let (mut graph, _supported) = independent_support_scenario();
    let unsupported = create_node(&mut graph, &["Claim"]);
    let post = create_node(&mut graph, &["Post"]);
    create_relationship(&mut graph, &post, "SUPPORTS", &unsupported);
    let facts = BitemporalFactStore::new();
    let as_of = ts("2026-06-01T00:00:00Z");

    let findings = validate_graph_epistemics(&empty_inputs(&graph, &facts, &as_of))
        .expect("epistemic validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-epistemic--unsupported-claim");
    assert_eq!(finding.severity(), ValidationErrorSeverity::Warning);
    assert_eq!(
        finding.target(),
        &ValidationTarget::node(unsupported.as_str())
    );
}

//
// Verify source-circularity detection: corroboration is pseudo-independent
// when every supporting observation of a claim traces back to the same single
// source.
//
// Given one claim whose two supporting observations are reported by the same
// source, next to the independent-support control,
// when epistemic validation runs,
// then exactly the circular claim should be flagged.
#[test]
fn source_circularity_is_detected() {
    let (mut graph, _independent) = independent_support_scenario();
    let circular_claim = create_node(&mut graph, &["Claim"]);
    let lone_source = create_node(&mut graph, &["Source"]);
    for _ in 0..2 {
        let observation = create_node(&mut graph, &["Observation"]);
        create_relationship(&mut graph, &lone_source, "REPORTS", &observation);
        create_relationship(&mut graph, &observation, "SUPPORTS", &circular_claim);
    }
    let facts = BitemporalFactStore::new();
    let as_of = ts("2026-06-01T00:00:00Z");

    let findings = validate_graph_epistemics(&empty_inputs(&graph, &facts, &as_of))
        .expect("epistemic validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-epistemic--source-circularity");
    assert_eq!(
        finding.target(),
        &ValidationTarget::node(circular_claim.as_str())
    );
    assert!(finding.message().contains(lone_source.as_str()));
}

//
// Verify open-contradiction detection: a CONTRADICTS relation without a
// declared resolution stays a typed finding, and declaring it resolved clears
// it.
//
// Given an observation contradicting a claim,
// when validation runs without and then with the contradiction declared
// resolved,
// then the first run should flag the relation and the second should not.
#[test]
fn open_contradictions_are_detected_until_resolved() {
    let (mut graph, claim) = independent_support_scenario();
    let source = create_node(&mut graph, &["Source"]);
    let observation = create_node(&mut graph, &["Observation"]);
    create_relationship(&mut graph, &source, "REPORTS", &observation);
    let contradicts = create_relationship(&mut graph, &observation, "CONTRADICTS", &claim);
    let facts = BitemporalFactStore::new();
    let as_of = ts("2026-06-01T00:00:00Z");

    let open = validate_graph_epistemics(&empty_inputs(&graph, &facts, &as_of))
        .expect("epistemic validation should run");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].code(), "immune-epistemic--open-contradiction");
    assert_eq!(
        open[0].target(),
        &ValidationTarget::relationship(contradicts.as_str())
    );

    let resolved_list = [contradicts];
    let resolved = validate_graph_epistemics(&EpistemicValidationInputs {
        graph: &graph,
        facts: &facts,
        as_of: &as_of,
        evidence_facts: &[],
        resolved_contradictions: &resolved_list,
    })
    .expect("epistemic validation should run");
    assert!(resolved.is_empty());
}

//
// Verify stale-evidence detection through the bitemporal as-of semantics: a
// support edge backed by a fact with no state valid at the as-of time is
// stale, while an open-ended fact stays fresh.
//
// Given two supported claims whose support edges are backed by a closed and
// an open-ended fact respectively,
// when validation runs as of a time after the closed interval,
// then exactly the stale support edge should be flagged.
#[test]
fn stale_evidence_is_detected_via_bitemporal_as_of() {
    let mut graph = Graph::new();
    let claim = create_node(&mut graph, &["Claim"]);
    let first_source = create_node(&mut graph, &["Source"]);
    let second_source = create_node(&mut graph, &["Source"]);
    let stale_observation = create_node(&mut graph, &["Observation"]);
    let fresh_observation = create_node(&mut graph, &["Observation"]);
    create_relationship(&mut graph, &first_source, "REPORTS", &stale_observation);
    create_relationship(&mut graph, &second_source, "REPORTS", &fresh_observation);
    let stale_support = create_relationship(&mut graph, &stale_observation, "SUPPORTS", &claim);
    let fresh_support = create_relationship(&mut graph, &fresh_observation, "SUPPORTS", &claim);

    let mut facts = BitemporalFactStore::new();
    let stale_fact = fact_id("fact--stale-infrastructure");
    let fresh_fact = fact_id("fact--fresh-infrastructure");
    facts
        .assert_fact_state(
            stale_fact.clone(),
            "Old infrastructure in use",
            BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-01-02T00:00:00Z"))
                .expect("stamp should be valid")
                .with_valid_to(ts("2026-02-01T00:00:00Z"))
                .expect("valid-to should follow valid-from"),
        )
        .expect("stale fact state should be asserted");
    facts
        .assert_fact_state(
            fresh_fact.clone(),
            "Current infrastructure in use",
            BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-01-02T00:00:00Z"))
                .expect("stamp should be valid"),
        )
        .expect("fresh fact state should be asserted");

    let as_of = ts("2026-06-01T00:00:00Z");
    let evidence_facts = [
        (stale_support.clone(), stale_fact),
        (fresh_support, fresh_fact),
    ];
    let findings = validate_graph_epistemics(&EpistemicValidationInputs {
        graph: &graph,
        facts: &facts,
        as_of: &as_of,
        evidence_facts: &evidence_facts,
        resolved_contradictions: &[],
    })
    .expect("epistemic validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-epistemic--stale-evidence");
    assert_eq!(
        finding.target(),
        &ValidationTarget::relationship(stale_support.as_str())
    );
}

//
// Verify deterministic reporting: findings follow the documented order —
// unsupported claims, source circularity, open contradictions, stale
// evidence — and identical inputs yield identical findings, without mutating
// the graph.
//
// Given a scenario seeding one defect of each class,
// when validation runs twice,
// then both runs should agree, follow the documented order, and leave the
// graph unchanged.
#[test]
fn findings_are_deterministically_ordered_and_pure() {
    let build = || {
        let mut graph = Graph::new();
        // Unsupported claim.
        let unsupported = create_node(&mut graph, &["Claim"]);
        let _ = unsupported;
        // Circular claim.
        let circular = create_node(&mut graph, &["Claim"]);
        let lone_source = create_node(&mut graph, &["Source"]);
        for _ in 0..2 {
            let observation = create_node(&mut graph, &["Observation"]);
            create_relationship(&mut graph, &lone_source, "REPORTS", &observation);
            create_relationship(&mut graph, &observation, "SUPPORTS", &circular);
        }
        // Open contradiction against the circular claim.
        let contradicting = create_node(&mut graph, &["Observation"]);
        create_relationship(&mut graph, &lone_source, "REPORTS", &contradicting);
        create_relationship(&mut graph, &contradicting, "CONTRADICTS", &circular);
        // Stale support backing.
        let stale_support = graph
            .list_relationships()
            .expect("relationships should list")
            .into_iter()
            .find(|relationship| relationship.rel_type().as_str() == "SUPPORTS")
            .expect("a support edge should exist")
            .id()
            .clone();

        let mut facts = BitemporalFactStore::new();
        let fact = fact_id("fact--ordering");
        facts
            .assert_fact_state(
                fact.clone(),
                "Closed interval",
                BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-01-02T00:00:00Z"))
                    .expect("stamp should be valid")
                    .with_valid_to(ts("2026-02-01T00:00:00Z"))
                    .expect("valid-to should follow valid-from"),
            )
            .expect("fact state should be asserted");
        (graph, facts, stale_support, fact)
    };

    let run = |(graph, facts, stale_support, fact): &(
        Graph,
        BitemporalFactStore,
        RelationshipId,
        FactId,
    )| {
        let as_of = ts("2026-06-01T00:00:00Z");
        let evidence_facts = [(stale_support.clone(), fact.clone())];
        validate_graph_epistemics(&EpistemicValidationInputs {
            graph,
            facts,
            as_of: &as_of,
            evidence_facts: &evidence_facts,
            resolved_contradictions: &[],
        })
        .expect("epistemic validation should run")
    };

    let first_build = build();
    let nodes_before = first_build.0.list_nodes().expect("nodes should list");
    let first = run(&first_build);
    assert_eq!(
        first_build.0.list_nodes().expect("nodes should list"),
        nodes_before,
        "validation must not mutate the graph"
    );

    let second = run(&build());
    assert_eq!(first, second);

    let codes: Vec<&str> = first.iter().map(|finding| finding.code()).collect();
    assert_eq!(
        codes,
        vec![
            "immune-epistemic--unsupported-claim",
            "immune-epistemic--source-circularity",
            "immune-epistemic--open-contradiction",
            "immune-epistemic--stale-evidence",
        ]
    );
}
