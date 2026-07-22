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
use cypher_parser::{
    Allowance, InvestigationErrorCode, InvestigationIntent, InvestigationTargetKind, Requirement,
    ReturnProjection, parse_investigation_query,
};

const COMPLETE_QUERY: &str = r#"
    INVESTIGATE attribution OF Campaign("C-42")
    AT TIME 2026-06-01
    REQUIRE independent_sources >= 2, source_reliability >= 0.70, evidence_completeness >= 0.80
    ALLOW hypotheses = true, contradictory_evidence = true
    BUDGET memory = 256 MB, latency = 3 s, external_retrievals = 4
    RETURN assessment, proof_graph, counter_evidence, unknowns, next_best_evidence
"#;

#[test]
fn parses_complete_investigation_contract_into_typed_ast() {
    let query =
        parse_investigation_query(COMPLETE_QUERY).expect("complete investigation should parse");

    assert_eq!(query.intent, InvestigationIntent::Attribution);
    assert_eq!(query.target.kind, InvestigationTargetKind::Campaign);
    assert_eq!(query.target.identifier, "C-42");
    assert_eq!(
        query.at_time.as_ref().map(|timestamp| timestamp.as_str()),
        Some("2026-06-01")
    );
    assert_eq!(
        query.requirements,
        vec![
            Requirement::IndependentSourcesAtLeast(2),
            Requirement::SourceReliabilityAtLeast(
                cypher_parser::NormalizedThreshold::from_parts_per_million(700_000)
                    .expect("threshold should be valid"),
            ),
            Requirement::EvidenceCompletenessAtLeast(
                cypher_parser::NormalizedThreshold::from_parts_per_million(800_000)
                    .expect("threshold should be valid"),
            ),
        ]
    );
    assert_eq!(
        query.allowances,
        vec![
            Allowance::Hypotheses(true),
            Allowance::ContradictoryEvidence(true),
        ]
    );

    let budget = query.budget.expect("budget should be present");
    assert_eq!(budget.memory_bytes, Some(256 * 1024 * 1024));
    assert_eq!(budget.latency_millis, Some(3_000));
    assert_eq!(budget.external_retrievals, Some(4));
    assert_eq!(
        query.returns,
        vec![
            ReturnProjection::Assessment,
            ReturnProjection::ProofGraph,
            ReturnProjection::CounterEvidence,
            ReturnProjection::Unknowns,
            ReturnProjection::NextBestEvidence,
        ]
    );
}

#[test]
fn partial_budget_preserves_only_the_declared_bound() {
    let query = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           BUDGET memory = 256 MB
           RETURN assessment"#,
    )
    .expect("a partial budget should parse without implicit limits");

    let budget = query.budget.expect("budget should be present");
    assert_eq!(budget.memory_bytes, Some(256 * 1024 * 1024));
    assert_eq!(budget.latency_millis, None);
    assert_eq!(budget.external_retrievals, None);
}

#[test]
fn equivalent_inputs_normalize_to_one_canonical_representation() {
    let first =
        parse_investigation_query(COMPLETE_QUERY).expect("complete investigation should parse");
    let reordered = parse_investigation_query(
        r#"investigate ATTRIBUTION of campaign("C-42")
           return next_best_evidence, unknowns, counter_evidence, proof_graph, assessment
           budget external_retrievals = 4, latency = 3000 ms, memory = 262144 KB
           allow contradictory_evidence = TRUE, hypotheses = TRUE
           require evidence_completeness >= 0.800000, source_reliability >= .7, independent_sources >= 2
           at time 2026-06-01"#,
    )
    .expect("equivalent investigation should parse");

    assert_eq!(first, reordered);
    assert_eq!(first.to_canonical_string(), reordered.to_canonical_string());
}

#[test]
fn duplicate_and_conflicting_contracts_are_rejected() {
    let duplicate_clause = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           BUDGET memory = 1 MB
           BUDGET latency = 1 s
           RETURN assessment"#,
    )
    .expect_err("duplicate budget clauses should fail");
    assert_eq!(
        duplicate_clause.code,
        InvestigationErrorCode::DuplicateClause
    );
    assert!(duplicate_clause.message.contains("BUDGET"));

    let conflicting_requirement = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           REQUIRE source_reliability >= 0.70, source_reliability >= 0.80
           RETURN assessment"#,
    )
    .expect_err("conflicting requirements should fail");
    assert_eq!(
        conflicting_requirement.code,
        InvestigationErrorCode::ConflictingContract
    );
    assert!(
        conflicting_requirement
            .message
            .contains("source_reliability")
    );
}

#[test]
fn missing_unsupported_and_invalid_values_have_typed_actionable_errors() {
    let missing_return =
        parse_investigation_query(r#"INVESTIGATE attribution OF Campaign("C-42")"#)
            .expect_err("RETURN is mandatory");
    assert_eq!(missing_return.code, InvestigationErrorCode::MissingClause);
    assert!(missing_return.suggestion.is_some());

    let unsupported_intent = parse_investigation_query(
        r#"INVESTIGATE prediction OF Campaign("C-42") RETURN assessment"#,
    )
    .expect_err("unsupported intent should fail");
    assert_eq!(
        unsupported_intent.code,
        InvestigationErrorCode::UnsupportedIntent
    );

    let unsupported_target = parse_investigation_query(
        r#"INVESTIGATE attribution OF Spreadsheet("C-42") RETURN assessment"#,
    )
    .expect_err("unsupported target should fail");
    assert_eq!(
        unsupported_target.code,
        InvestigationErrorCode::UnsupportedTarget
    );

    let invalid_threshold = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           REQUIRE source_reliability >= 1.20
           RETURN assessment"#,
    )
    .expect_err("threshold above one should fail");
    assert_eq!(invalid_threshold.code, InvestigationErrorCode::InvalidValue);
    assert!(invalid_threshold.message.contains("source_reliability"));
}

#[test]
fn lower_level_cypher_cannot_be_embedded_in_investigation_statements() {
    let error = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           RETURN assessment
           MATCH (n) RETURN n"#,
    )
    .expect_err("embedded Cypher should not bypass the gateway");

    assert_eq!(error.code, InvestigationErrorCode::GatewayBoundaryViolation);
    assert!(error.message.contains("MATCH"));
    assert!(error.suggestion.is_some());
}

#[test]
fn malformed_temporal_and_resource_units_are_rejected() {
    let invalid_date = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           AT TIME 2026-13-01
           RETURN assessment"#,
    )
    .expect_err("invalid date should fail");
    assert_eq!(invalid_date.code, InvestigationErrorCode::InvalidValue);

    let invalid_memory = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           BUDGET memory = 256 XB
           RETURN assessment"#,
    )
    .expect_err("unsupported memory unit should fail");
    assert_eq!(invalid_memory.code, InvestigationErrorCode::InvalidValue);
    assert!(invalid_memory.message.contains("memory"));
}
