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
use cypher_parser::parse_query;
use cypher_planner::{PlanOperator, build_logical_plan};

#[test]
fn build_logical_plan_adds_operators_deterministically_for_read_pipeline() {
    let ast = parse_query("MATCH (n) WHERE n.kind = 'indicator' RETURN n LIMIT 5")
        .expect("query should parse");

    let plan = build_logical_plan(&ast);

    assert_eq!(
        plan.operators,
        vec![
            PlanOperator::NodeScan,
            PlanOperator::Filter,
            PlanOperator::Projection,
            PlanOperator::Limit,
        ]
    );
}

#[test]
fn build_logical_plan_adds_expand_operator_for_relationship_match() {
    let ast = parse_query("MATCH (a:Actor)-[:AMPLIFIES]->(n:Narrative) RETURN a, n")
        .expect("relationship query should parse");

    let plan = build_logical_plan(&ast);

    assert_eq!(
        plan.operators,
        vec![
            PlanOperator::NodeScan,
            PlanOperator::ExpandRelationship,
            PlanOperator::Projection,
        ]
    );
}

#[test]
fn build_logical_plan_adds_mutation_operator_for_create_query() {
    let ast = parse_query("CREATE (n:Indicator {name: 'x'})").expect("CREATE query should parse");

    let plan = build_logical_plan(&ast);

    assert!(
        plan.operators.contains(&PlanOperator::Mutation),
        "mutation query should include Mutation operator"
    );
}

#[test]
fn build_logical_plan_adds_mutation_and_read_operators_for_mixed_query() {
    let ast = parse_query("MATCH (n:Indicator) WHERE n.name = 'x' SET n.score = 1 RETURN n")
        .expect("mixed query should parse");

    let plan = build_logical_plan(&ast);

    assert!(
        plan.operators.contains(&PlanOperator::NodeScan),
        "mixed query should include NodeScan"
    );
    assert!(
        plan.operators.contains(&PlanOperator::Mutation),
        "mixed query should include Mutation"
    );
    assert!(
        plan.operators.contains(&PlanOperator::Projection),
        "mixed query with RETURN should include Projection"
    );
}
