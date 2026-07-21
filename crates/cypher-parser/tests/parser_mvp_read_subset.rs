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
use cypher_parser::{AggregationFunction, ClauseKind, ParseErrorCode, parse_query};

#[test]
fn parse_query_extracts_supported_read_clauses_in_order() {
    let ast = parse_query(
        "MATCH (n:Indicator) WHERE n.score > 10 RETURN n ORDER BY n.score SKIP 2 LIMIT 5",
    )
    .expect("supported read query should parse");

    assert_eq!(
        ast.clauses,
        vec![
            ClauseKind::Match,
            ClauseKind::Where,
            ClauseKind::Return,
            ClauseKind::OrderBy,
            ClauseKind::Skip,
            ClauseKind::Limit,
        ]
    );
}

#[test]
fn parse_query_extracts_optional_match_with_and_distinct_clauses() {
    let ast = parse_query("OPTIONAL MATCH (n)-[:REL]->(m) WITH DISTINCT n RETURN n")
        .expect("supported query with optional match and with distinct should parse");

    assert_eq!(
        ast.clauses,
        vec![
            ClauseKind::OptionalMatch,
            ClauseKind::With,
            ClauseKind::Distinct,
            ClauseKind::Return,
        ]
    );
}

#[test]
fn parse_query_rejects_subquery_blocks_for_mvp_subset() {
    let error = parse_query("MATCH (n) CALL { WITH n RETURN n } RETURN n")
        .expect_err("subquery blocks should be rejected in mvp subset");

    assert_eq!(error.code, ParseErrorCode::UnsupportedFeature);
    assert!(error.message.contains("CALL {"));
}

#[test]
fn parse_query_extracts_basic_aggregation_functions_deterministically() {
    let ast = parse_query(
        "MATCH (n) RETURN COUNT(n), SUM(n.score), AVG(n.score), MIN(n.score), MAX(n.score)",
    )
    .expect("basic aggregation query should parse");

    assert_eq!(
        ast.aggregations,
        vec![
            AggregationFunction::Count,
            AggregationFunction::Sum,
            AggregationFunction::Avg,
            AggregationFunction::Min,
            AggregationFunction::Max,
        ]
    );
}

#[test]
fn parse_query_aggregation_detection_is_case_insensitive() {
    let ast = parse_query("MATCH (n) RETURN count(n), SuM(n.score)")
        .expect("aggregation detection should be case-insensitive");

    assert_eq!(
        ast.aggregations,
        vec![AggregationFunction::Count, AggregationFunction::Sum]
    );
}

#[test]
fn parse_query_builds_structured_match_where_and_return_ast() {
    let ast = parse_query("MATCH (n:Indicator) WHERE n.score > 10 RETURN n LIMIT 3")
        .expect("supported mvp read subset should parse to structured ast");

    let query = ast.query.expect("query payload should be present");
    let match_clause = query
        .match_clause
        .expect("match clause should be parsed for read subset");
    assert_eq!(match_clause.start.variable, "n");
    assert_eq!(match_clause.start.label.as_deref(), Some("Indicator"));
    assert!(match_clause.relationship.is_none());

    let where_clause = query.where_clause.expect("where clause should be parsed");
    assert_eq!(where_clause.left.variable, "n");
    assert_eq!(where_clause.left.property, "score");

    let return_clause = query.return_clause.expect("return clause should be parsed");
    assert_eq!(return_clause.items.len(), 1);
    assert_eq!(return_clause.limit, Some(3));
}

#[test]
fn parse_query_builds_structured_relationship_pattern_ast() {
    let ast = parse_query("MATCH (a:Actor)-[:AMPLIFIES]->(n:Narrative) RETURN a, n")
        .expect("relationship match pattern should parse");

    let query = ast.query.expect("query payload should be present");
    let match_clause = query.match_clause.expect("match clause should be present");
    let relationship = match_clause
        .relationship
        .expect("relationship pattern should be captured");

    assert_eq!(relationship.0.rel_type.as_deref(), Some("AMPLIFIES"));
    assert_eq!(relationship.1.variable, "n");
    assert_eq!(relationship.1.label.as_deref(), Some("Narrative"));
}

#[test]
fn parse_query_degrades_to_unstructured_ast_when_where_is_after_return() {
    let ast = parse_query("MATCH (n:Indicator) RETURN n WHERE n.score > 10")
        .expect("query should still produce top-level ast");

    assert!(ast.query.is_none());
    assert!(ast.clauses.contains(&ClauseKind::Match));
    assert!(ast.clauses.contains(&ClauseKind::Return));
    assert!(ast.clauses.contains(&ClauseKind::Where));
}

#[test]
fn parse_query_degrades_to_unstructured_ast_when_order_direction_is_invalid() {
    let ast = parse_query("MATCH (n) RETURN n ORDER BY n.score SIDEWAYS")
        .expect("query should still produce top-level ast");

    assert!(ast.query.is_none());
    assert!(ast.clauses.contains(&ClauseKind::OrderBy));
}

#[test]
fn parse_query_degrades_to_unstructured_ast_when_skip_or_limit_are_invalid() {
    let invalid_skip = parse_query("MATCH (n) RETURN n SKIP nope")
        .expect("query should still produce top-level ast");
    assert!(invalid_skip.query.is_none());
    assert!(invalid_skip.clauses.contains(&ClauseKind::Skip));

    let invalid_limit = parse_query("MATCH (n) RETURN n LIMIT nope")
        .expect("query should still produce top-level ast");
    assert!(invalid_limit.query.is_none());
    assert!(invalid_limit.clauses.contains(&ClauseKind::Limit));
}

#[test]
fn parse_query_parses_not_equal_operator_in_structured_where_clause() {
    let ast = parse_query("MATCH (n:Indicator) WHERE n.score <> 10 RETURN n")
        .expect("not-equal where query should parse");

    let query = ast.query.expect("structured query should be present");
    let where_clause = query.where_clause.expect("where clause should be present");
    assert_eq!(
        where_clause.operator,
        cypher_parser::ComparisonOperator::NotEq
    );
}

#[test]
fn parse_query_degrades_to_unstructured_ast_for_malformed_match_pattern_shapes() {
    let missing_opening_paren =
        parse_query("MATCH n RETURN n").expect("query should still return top-level ast");
    assert!(missing_opening_paren.query.is_none());

    let missing_outgoing_arrow = parse_query("MATCH (n)-[:REL](m) RETURN n")
        .expect("query should still return top-level ast");
    assert!(missing_outgoing_arrow.query.is_none());

    let wrong_relationship_prefix = parse_query("MATCH (n) [r]->(m) RETURN n")
        .expect("query should still return top-level ast");
    assert!(wrong_relationship_prefix.query.is_none());
}

#[test]
fn parse_query_degrades_to_unstructured_ast_for_invalid_where_literal_or_reference() {
    let invalid_property_ref = parse_query("MATCH (n) WHERE n = 10 RETURN n")
        .expect("query should still return top-level ast");
    assert!(invalid_property_ref.query.is_none());

    let unsupported_literal = parse_query("MATCH (n) WHERE n.score = 10.5 RETURN n")
        .expect("query should still return top-level ast");
    assert!(unsupported_literal.query.is_none());
}

#[test]
fn parse_query_degrades_to_unstructured_ast_for_invalid_return_clause_shapes() {
    let empty_projection =
        parse_query("MATCH (n) RETURN").expect("query should still return top-level ast");
    assert!(empty_projection.query.is_none());

    let order_without_field = parse_query("MATCH (n) RETURN n ORDER BY")
        .expect("query should still return top-level ast");
    assert!(order_without_field.query.is_none());

    let unsupported_tail_token = parse_query("MATCH (n) RETURN n SKIP 1 OOPS")
        .expect("query should still return top-level ast");
    assert!(unsupported_tail_token.query.is_none());
}
