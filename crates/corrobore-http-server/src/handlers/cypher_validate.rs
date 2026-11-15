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

//! Parse-only Cypher validation for editors (issue #336).
//!
//! The route only parses: it never builds a projection, never touches the
//! engine and never writes, so an editor may call it on every keystroke.

use axum::Json;
use cypher_parser::{ClauseKind, ParseErrorCode, QueryKind, parse_query};
use cypher_planner::{PlanOperator, PlannedElement, build_logical_plan};
use serde::{Deserialize, Serialize};

use crate::{
    error::ApiError,
    handlers::graph_history::{Envelope, ok},
};

const MAX_VALIDATED_QUERY_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
pub struct ValidateCypherRequest {
    pub query: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CypherDiagnosticView {
    /// `error` blocks running the query; `warning` does not.
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ValidateCypherResult {
    pub valid: bool,
    /// `read`, `mutation` or `mixed`; null when the query does not parse.
    pub kind: Option<&'static str>,
    pub clauses: Vec<&'static str>,
    pub diagnostics: Vec<CypherDiagnosticView>,
    /// Logical plan of a parsed query (issue #360); null otherwise.
    pub plan: Option<PlanView>,
}

/// The logical plan Corrobore builds for a query, for Explain.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PlanView {
    /// Operators in execution order.
    pub operators: Vec<&'static str>,
    /// `read`, `mutation` or `mixed`.
    pub query_kind: &'static str,
    /// Pattern variables, what they bind and whether they are returned.
    pub bindings: Vec<PlanBindingView>,
}

/// One pattern variable of a plan.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PlanBindingView {
    pub variable: String,
    /// `node` or `relationship`.
    pub element: &'static str,
    pub projected: bool,
}

fn operator_name(operator: &PlanOperator) -> &'static str {
    match operator {
        PlanOperator::NodeScan => "node_scan",
        PlanOperator::ExpandRelationship => "expand_relationship",
        PlanOperator::Filter => "filter",
        PlanOperator::Projection => "projection",
        PlanOperator::Sort => "sort",
        PlanOperator::Skip => "skip",
        PlanOperator::Limit => "limit",
        PlanOperator::Mutation => "mutation",
        PlanOperator::FunctionCall => "function_call",
    }
}

fn plan_view(ast: &cypher_parser::QueryAst) -> PlanView {
    let plan = build_logical_plan(ast);
    PlanView {
        operators: plan.operators.iter().map(operator_name).collect(),
        query_kind: kind_name(&plan.query_kind),
        bindings: plan
            .provenance
            .declared_bindings()
            .iter()
            .map(|binding| PlanBindingView {
                variable: binding.variable().to_owned(),
                element: match binding.element() {
                    PlannedElement::Node => "node",
                    PlannedElement::Relationship => "relationship",
                },
                projected: binding.projected(),
            })
            .collect(),
    }
}

fn kind_name(kind: &QueryKind) -> &'static str {
    match kind {
        QueryKind::Read => "read",
        QueryKind::Mutation => "mutation",
        QueryKind::Mixed => "mixed",
    }
}

fn clause_name(clause: &ClauseKind) -> &'static str {
    match clause {
        ClauseKind::Match => "MATCH",
        ClauseKind::OptionalMatch => "OPTIONAL MATCH",
        ClauseKind::Where => "WHERE",
        ClauseKind::Return => "RETURN",
        ClauseKind::With => "WITH",
        ClauseKind::Distinct => "DISTINCT",
        ClauseKind::OrderBy => "ORDER BY",
        ClauseKind::Skip => "SKIP",
        ClauseKind::Limit => "LIMIT",
        ClauseKind::Create => "CREATE",
        ClauseKind::Merge => "MERGE",
        ClauseKind::Set => "SET",
        ClauseKind::Remove => "REMOVE",
        ClauseKind::Delete => "DELETE",
    }
}

/// Whether the text writes, judged by its keywords, for queries whose
/// parameters are not bound yet.
fn writes_by_keyword(query: &str) -> bool {
    query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| {
            matches!(
                word.to_ascii_uppercase().as_str(),
                "CREATE" | "MERGE" | "SET" | "DELETE" | "REMOVE" | "DETACH"
            )
        })
}

/// Validate a query without executing it.
pub fn validate_query(query: &str) -> ValidateCypherResult {
    match parse_query(query) {
        Ok(ast) => {
            let mut diagnostics = Vec::new();
            if ast.query.is_none() && ast.kind == QueryKind::Read {
                // The parser recognised the clauses but not their structure.
                diagnostics.push(CypherDiagnosticView {
                    severity: "warning",
                    code: "UNSTRUCTURED_QUERY",
                    message: "Corrobore recognised the clauses but not their structure, \
                              so it cannot narrow the records it loads for this query."
                        .to_owned(),
                    suggestion: Some(
                        "Check literals such as LIMIT and SKIP values and property references."
                            .to_owned(),
                    ),
                });
            }
            ValidateCypherResult {
                valid: true,
                kind: Some(kind_name(&ast.kind)),
                clauses: ast.clauses.iter().map(clause_name).collect(),
                diagnostics,
                plan: Some(plan_view(&ast)),
            }
        }
        Err(error) if error.code == ParseErrorCode::InvalidParameter => ValidateCypherResult {
            // Parameters are bound when the query runs; while typing this is
            // only a reminder.
            valid: true,
            kind: Some(if writes_by_keyword(query) {
                "mutation"
            } else {
                "read"
            }),
            clauses: Vec::new(),
            diagnostics: vec![CypherDiagnosticView {
                severity: "warning",
                code: "UNBOUND_PARAMETER",
                message: error.message,
                suggestion: error.suggestion,
            }],
            plan: None,
        },
        Err(error) => ValidateCypherResult {
            valid: false,
            kind: None,
            clauses: Vec::new(),
            diagnostics: vec![CypherDiagnosticView {
                severity: "error",
                code: match error.code {
                    ParseErrorCode::EmptyQuery => "EMPTY_QUERY",
                    ParseErrorCode::UnsupportedFeature => "UNSUPPORTED_FEATURE",
                    ParseErrorCode::InvalidSyntax => "INVALID_SYNTAX",
                    ParseErrorCode::InvalidParameter => "INVALID_PARAMETER",
                },
                message: error.message,
                suggestion: error.suggestion,
            }],
            plan: None,
        },
    }
}

/// `POST /v1/cypher/validate`: parse a query and report diagnostics; never executes.
pub async fn validate_cypher(
    Json(request): Json<ValidateCypherRequest>,
) -> Result<Json<Envelope<ValidateCypherResult>>, ApiError> {
    if request.query.len() > MAX_VALIDATED_QUERY_BYTES {
        return Err(ApiError::bad_request(
            "QUERY_TOO_LARGE",
            "queries longer than 64 KiB are not validated",
        ));
    }
    Ok(ok(validate_query(&request.query)))
}
