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
#![allow(clippy::unwrap_used)]
//! A query answer says what it read and, separately, what supports it.
//!
//! Computational provenance is causal: the query used this substructure.
//! Semantic support is evidential: this evidence supports the claim. Keeping
//! them in one field would let a reader take "the query touched it" for "the
//! evidence backs it", which is the confusion this contract exists to prevent.
use cypher_executor::{CypherPipelineExecutor, ExecutionPolicy, ProvenanceElement};
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimTarget, EpistemicStores, EvidenceRecordStore,
    EvidenceSourceType, Graph, NodeInput, ObservationId, ObservationInput, ObservationModality,
    PropertyValue, RecordStatus, RelationshipInput, ResolutionInputs, SourceId, SourceInput,
    TemporalTimestamp, resolve_claim_verdict,
};

fn linked_graph() -> Graph {
    let mut graph = Graph::new();
    let actor = graph
        .create_node(
            NodeInput::new(["Actor"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("alpha".to_owned()))
                .with_property("weight", PropertyValue::Integer(3)),
        )
        .expect("actor");
    let narrative = graph
        .create_node(
            NodeInput::new(["Narrative"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("n1".to_owned())),
        )
        .expect("narrative");
    graph
        .create_relationship(
            RelationshipInput::new(actor, "AMPLIFIES", narrative)
                .expect("relationship input")
                .with_status(RecordStatus::Exportable),
        )
        .expect("relationship");
    graph
}

fn claim_projection() -> Graph {
    let mut stores = EpistemicStores::default();
    stores
        .sources
        .register_source(SourceInput::new(
            SourceId::new("source--report").unwrap(),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    let observation = ObservationId::new("observation--span").unwrap();
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                SourceId::new("source--report").unwrap(),
                "the recorded span",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .unwrap();
    let claim = ClaimId::new("claim--provenance").unwrap();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("the span was recorded").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("span", None)),
        ))
        .unwrap();
    stores.claims.register_observation(observation.clone());
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(observation),
            claim.clone(),
            ClaimLinkKind::Supports,
        ))
        .unwrap();
    let evidence = EvidenceRecordStore::new();
    let mut claims = std::mem::take(&mut stores.claims);
    let mut verdicts = std::mem::take(&mut stores.verdicts);
    resolve_claim_verdict(
        &mut claims,
        &mut verdicts,
        &ResolutionInputs::new(
            &stores.verifications,
            &evidence,
            &stores.observations,
            &stores.sources,
        ),
        &claim,
        BitemporalStamp::new(
            TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap(),
            TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap(),
        )
        .unwrap(),
        "ws-a-minimal-v1",
    )
    .unwrap();
    stores.claims = claims;
    stores.verdicts = verdicts;
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores);
    graph.epistemic_projection().unwrap()
}

fn run(graph: Graph, query: &str) -> cypher_executor::ExecutionResult {
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);
    executor.execute(query).expect("query should execute")
}

fn ids(provenance: &cypher_executor::RowProvenance) -> Vec<(String, String)> {
    provenance
        .contributing()
        .iter()
        .map(|element| {
            (
                element.variable().to_owned(),
                match element.element() {
                    ProvenanceElement::Node(id) => format!("node:{id}"),
                    ProvenanceElement::Relationship(id) => format!("relationship:{id}"),
                },
            )
        })
        .collect()
}

//
// A result exposes the substructure it read, per row, without any manual
// tracing: the executor records what it bound while it bound it.
#[test]
fn a_query_result_exposes_its_contributing_substructure_without_manual_tracing() {
    let result = run(
        linked_graph(),
        "MATCH (a:Actor)-[r:AMPLIFIES]->(b:Narrative) RETURN a, r",
    );
    let provenance = result
        .why_provenance
        .as_ref()
        .expect("a read result carries why-provenance");

    assert_eq!(provenance.computational().len(), 1);
    let row = &provenance.computational()[0];
    assert_eq!(row.row(), 0);
    let contributed = ids(row);
    assert_eq!(
        contributed.len(),
        3,
        "both endpoints and the edge were read"
    );
    assert!(
        contributed
            .iter()
            .any(|(variable, element)| variable == "r" && element.starts_with("relationship:"))
    );
    assert!(
        row.contributing()
            .iter()
            .filter(|element| element.projected())
            .count()
            == 2,
        "only the projected bindings reach the answer"
    );

    // The plan declared the read set before execution measured it.
    assert_eq!(
        provenance
            .declared()
            .declared_bindings()
            .iter()
            .map(|binding| binding.variable().to_owned())
            .collect::<Vec<_>>(),
        ["a", "r", "b"]
    );
}

//
// Computational and semantic provenance are separate: a graph with no claim
// says only what the query used, and says so explicitly.
#[test]
fn a_result_without_claims_reports_computational_provenance_only() {
    let result = run(linked_graph(), "MATCH (a:Actor) RETURN a");
    let provenance = result.why_provenance.expect("why-provenance");

    assert!(provenance.semantic_support().is_empty());
    assert!(provenance.is_computational_only());
    assert_eq!(provenance.computational().len(), 1);
}

//
// A computed field shows both its causal inputs and the evidence that
// semantically supports the claim behind them, in distinct fields.
#[test]
fn a_computed_field_shows_its_causal_inputs_and_its_semantic_support_distinctly() {
    let result = run(claim_projection(), "MATCH (c:Claim) RETURN count(c)");
    let provenance = result.why_provenance.expect("why-provenance");

    assert_eq!(
        provenance.computational().len(),
        1,
        "one aggregate answer has one causal input set"
    );
    let causal = &provenance.computational()[0];
    assert!(!causal.contributing().is_empty());
    assert!(!provenance.is_computational_only());

    let support = provenance.semantic_support();
    assert_eq!(support.len(), 1);
    assert_eq!(support[0].claim_id(), "claim--provenance");
    assert_eq!(support[0].verdict_state(), Some("supported"));
    assert_eq!(
        support[0].supporting().len(),
        1,
        "the supporting observation is evidential, not merely read"
    );
    assert!(support[0].refuting().is_empty());

    // The claim node appears on both sides with different meanings, and the two
    // are never merged into one list.
    let read_nodes: Vec<String> = causal
        .contributing()
        .iter()
        .filter_map(|element| match element.element() {
            ProvenanceElement::Node(id) => Some(id.to_owned()),
            ProvenanceElement::Relationship(_) => None,
        })
        .collect();
    assert!(read_nodes.contains(&support[0].node_id().to_owned()));
    assert!(
        !read_nodes.contains(&support[0].supporting()[0].to_owned()),
        "the supporting observation was never read by this query"
    );
}

//
// Provenance follows the answer: aggregation keeps every row that fed it, and a
// query that returns nothing claims no provenance.
#[test]
fn provenance_covers_every_row_that_fed_an_aggregate_and_is_absent_without_records() {
    let mut graph = linked_graph();
    graph
        .create_node(
            NodeInput::new(["Actor"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("beta".to_owned()))
                .with_property("weight", PropertyValue::Integer(4)),
        )
        .expect("second actor");

    let aggregate = run(graph.clone(), "MATCH (n:Actor) RETURN sum(n.weight)")
        .why_provenance
        .expect("why-provenance");
    assert_eq!(aggregate.computational().len(), 1);
    assert_eq!(
        aggregate.computational()[0].contributing().len(),
        2,
        "both rows fed the sum"
    );

    let empty = run(graph, "MATCH (n:Missing) RETURN n")
        .why_provenance
        .expect("why-provenance");
    assert!(empty.computational().is_empty());
    assert!(empty.is_computational_only());
}
