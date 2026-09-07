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

/// What kind of element a declared binding will hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlannedElement {
    /// A node pattern.
    Node,
    /// A relationship pattern.
    Relationship,
}

/// One binding the plan will produce while matching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedBinding {
    variable: String,
    element: PlannedElement,
    projected: bool,
}

/// The read set a query declares before it runs.
///
/// This is the planner half of why-provenance: the plan states what will be
/// bound and which bindings reach the answer, so the executor records a
/// measurement against a declaration instead of tracing a query by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvenancePlan {
    declared_bindings: Vec<PlannedBinding>,
}

impl PlannedBinding {
    /// Pattern variable.
    pub fn variable(&self) -> &str {
        &self.variable
    }

    /// Kind of element the binding holds.
    pub fn element(&self) -> PlannedElement {
        self.element
    }

    /// Whether the answer reads this binding, directly or through a computed
    /// field. A binding that is matched but never read is declared and not
    /// projected.
    pub fn projected(&self) -> bool {
        self.projected
    }
}

impl ProvenancePlan {
    /// Every declared binding in pattern order.
    pub fn declared_bindings(&self) -> &[PlannedBinding] {
        &self.declared_bindings
    }

    /// One declared binding by variable.
    pub fn binding(&self, variable: &str) -> Option<&PlannedBinding> {
        self.declared_bindings
            .iter()
            .find(|binding| binding.variable == variable)
    }

    /// Variables the answer reads, in pattern order.
    pub fn projected_variables(&self) -> Vec<&str> {
        self.declared_bindings
            .iter()
            .filter(|binding| binding.projected)
            .map(|binding| binding.variable.as_str())
            .collect()
    }
}

// A projection item reads exactly one variable, whether it names it directly or
// computes over one of its properties.
fn projected_variable(item: &cypher_parser::ProjectionItem) -> &str {
    match item {
        cypher_parser::ProjectionItem::Variable(variable)
        | cypher_parser::ProjectionItem::Count(variable) => variable.as_str(),
        cypher_parser::ProjectionItem::Property(reference)
        | cypher_parser::ProjectionItem::Sum(reference)
        | cypher_parser::ProjectionItem::Average(reference)
        | cypher_parser::ProjectionItem::Minimum(reference)
        | cypher_parser::ProjectionItem::Maximum(reference) => reference.variable.as_str(),
    }
}

// Declare the read set from the match pattern, then mark the bindings the
// return clause reads. Order follows the pattern so a plan is stable.
fn build_provenance_plan(ast: &QueryAst) -> ProvenancePlan {
    let Some(match_clause) = ast
        .query
        .as_ref()
        .and_then(|query| query.match_clause.as_ref())
    else {
        return ProvenancePlan::default();
    };
    let projected: Vec<&str> = ast
        .query
        .as_ref()
        .and_then(|query| query.return_clause.as_ref())
        .map(|clause| clause.items.iter().map(projected_variable).collect())
        .unwrap_or_default();

    let mut declared: Vec<PlannedBinding> = Vec::new();
    let mut push = |variable: &str, element: PlannedElement| {
        if variable.trim().is_empty() || declared.iter().any(|b| b.variable == variable) {
            return;
        }
        declared.push(PlannedBinding {
            variable: variable.to_owned(),
            element,
            projected: projected.contains(&variable),
        });
    };
    push(&match_clause.start.variable, PlannedElement::Node);
    if let Some((relationship, target)) = &match_clause.relationship {
        if let Some(variable) = &relationship.variable {
            push(variable, PlannedElement::Relationship);
        }
        push(&target.variable, PlannedElement::Node);
    }
    for node in &match_clause.additional_nodes {
        push(&node.variable, PlannedElement::Node);
    }
    ProvenancePlan {
        declared_bindings: declared,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Logical plan.
pub struct LogicalPlan {
    /// Operators.
    pub operators: Vec<PlanOperator>,
    /// Query kind.
    pub query_kind: QueryKind,
    /// Read set this plan declares for why-provenance.
    pub provenance: ProvenancePlan,
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
        .is_some_and(|return_clause| !return_clause.order_by.is_empty())
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
        provenance: build_provenance_plan(ast),
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
        // A direct function call binds no graph pattern.
        provenance: ProvenancePlan::default(),
    }
}
