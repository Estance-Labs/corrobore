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
#![warn(missing_docs)]

//! Logical query planner for parsed Cypher ASTs.
//!
//! Translates a [`QueryAst`] into a [`LogicalPlan`] composed of ordered
//! operators (node scan, relationship expansion, filter, projection, sort,
//! skip, limit, mutation, function call).

use cypher_parser::{QueryAst, QueryKind};

mod investigation_plan;

pub use investigation_plan::*;

#[derive(Clone, Debug, PartialEq, Eq)]
/// Plan operator.
pub enum PlanOperator {
    /// Node scan.
    NodeScan,
    /// Expand relationship.
    ExpandRelationship,
    /// Filter.
    Filter,
    /// Projection.
    Projection,
    /// Sort.
    Sort,
    /// Skip.
    Skip,
    /// Limit.
    Limit,
    /// Mutation.
    Mutation,
    /// Function call.
    FunctionCall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Logical plan.
pub struct LogicalPlan {
    /// Operators.
    pub operators: Vec<PlanOperator>,
    /// Query kind.
    pub query_kind: QueryKind,
}

//
// Build a deterministic logical plan skeleton from the parser AST.
// Operator selection is intentionally conservative and predictable for tests.
/// Creates the logical plan.
pub fn build_logical_plan(ast: &QueryAst) -> LogicalPlan {
    let mut operators = Vec::new();

    let has_match = ast.clauses.iter().any(|clause| {
        matches!(
            clause,
            cypher_parser::ClauseKind::Match | cypher_parser::ClauseKind::OptionalMatch
        )
    });

    if has_match {
        operators.push(PlanOperator::NodeScan);
    }

    if ast
        .query
        .as_ref()
        .and_then(|query| query.match_clause.as_ref())
        .and_then(|match_clause| match_clause.relationship.as_ref())
        .is_some()
    {
        operators.push(PlanOperator::ExpandRelationship);
    }

    let has_where = ast
        .query
        .as_ref()
        .and_then(|query| query.where_clause.as_ref())
        .is_some()
        || ast
            .clauses
            .iter()
            .any(|clause| matches!(clause, cypher_parser::ClauseKind::Where));
    if has_where {
        operators.push(PlanOperator::Filter);
    }

    let has_projection = ast.clauses.iter().any(|clause| {
        matches!(
            clause,
            cypher_parser::ClauseKind::Return | cypher_parser::ClauseKind::With
        )
    });
    if has_projection {
        operators.push(PlanOperator::Projection);
    }

    let has_order = ast
        .query
        .as_ref()
        .and_then(|query| query.return_clause.as_ref())
        .and_then(|return_clause| return_clause.order_by.as_ref())
        .is_some()
        || ast
            .clauses
            .iter()
            .any(|clause| matches!(clause, cypher_parser::ClauseKind::OrderBy));
    if has_order {
        operators.push(PlanOperator::Sort);
    }

    let has_skip = ast
        .query
        .as_ref()
        .and_then(|query| query.return_clause.as_ref())
        .and_then(|return_clause| return_clause.skip)
        .is_some()
        || ast
            .clauses
            .iter()
            .any(|clause| matches!(clause, cypher_parser::ClauseKind::Skip));
    if has_skip {
        operators.push(PlanOperator::Skip);
    }

    let has_limit = ast
        .query
        .as_ref()
        .and_then(|query| query.return_clause.as_ref())
        .and_then(|return_clause| return_clause.limit)
        .is_some()
        || ast
            .clauses
            .iter()
            .any(|clause| matches!(clause, cypher_parser::ClauseKind::Limit));
    if has_limit {
        operators.push(PlanOperator::Limit);
    }

    if matches!(ast.kind, QueryKind::Mutation | QueryKind::Mixed) {
        operators.push(PlanOperator::Mutation);
    }

    LogicalPlan {
        operators,
        // Query kind.
        query_kind: ast.kind.clone(),
    }
}

// Build a deterministic plan shape for direct function invocation through the
// planner/executor boundary.
/// Creates the function call plan.
pub fn build_function_call_plan(_function_name: &str) -> LogicalPlan {
    LogicalPlan {
        // Operators.
        operators: vec![PlanOperator::FunctionCall],
        // Query kind.
        query_kind: QueryKind::Read,
    }
}
