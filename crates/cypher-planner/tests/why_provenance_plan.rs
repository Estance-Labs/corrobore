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
//! Why-provenance is planned, not traced by hand: the plan declares the read
//! set a query will bind and which of it reaches the answer.
use cypher_parser::parse_query;
use cypher_planner::{PlannedElement, build_logical_plan};

fn plan(query: &str) -> cypher_planner::ProvenancePlan {
    build_logical_plan(&parse_query(query).expect("query should parse")).provenance
}

#[test]
fn the_plan_declares_every_binding_a_read_query_will_contribute_from() {
    let provenance = plan("MATCH (a:Actor)-[r:LINKED_TO]->(b:Narrative) RETURN a, r");

    assert_eq!(
        provenance
            .declared_bindings()
            .iter()
            .map(|binding| (binding.variable(), binding.element(), binding.projected()))
            .collect::<Vec<_>>(),
        vec![
            ("a", PlannedElement::Node, true),
            ("r", PlannedElement::Relationship, true),
            ("b", PlannedElement::Node, false),
        ]
    );
    assert_eq!(provenance.projected_variables(), ["a", "r"]);
    assert!(provenance.binding("b").is_some_and(|binding| !binding.projected()));
    assert!(provenance.binding("absent").is_none());
}

#[test]
fn a_computed_field_declares_the_binding_it_reads() {
    let counted = plan("MATCH (n:Actor) RETURN count(n)");
    assert!(counted.binding("n").is_some_and(|binding| binding.projected()));

    let summed = plan("MATCH (n:Actor)-[r:LINKED_TO]->(m:Actor) RETURN sum(n.weight)");
    assert!(summed.binding("n").is_some_and(|binding| binding.projected()));
    assert!(
        summed.binding("m").is_some_and(|binding| !binding.projected()),
        "a binding the answer never reads is declared and not projected"
    );
    assert!(summed.binding("r").is_some_and(|binding| !binding.projected()));

    let property = plan("MATCH (n:Actor) RETURN n.name");
    assert!(property.binding("n").is_some_and(|binding| binding.projected()));
}

#[test]
fn an_unnamed_relationship_and_extra_patterns_are_still_declared() {
    let anonymous = plan("MATCH (a:Actor)-[:LINKED_TO]->(b:Actor) RETURN a");
    assert_eq!(anonymous.declared_bindings().len(), 2);
    assert!(anonymous.binding("a").is_some());
    assert!(anonymous.binding("b").is_some());

    let mutation = plan("CREATE (n:Actor)");
    assert!(
        mutation.declared_bindings().is_empty(),
        "a mutation without a match declares no read set"
    );
}
