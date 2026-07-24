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

//! Cypher query execution pipeline and policy enforcement.
//!
//! Translates logical plans into executable operations against the in-memory
//! graph, enforcing execution policies (read-only, mutation, mixed) and
//! collecting execution records for auditability.

use std::collections::HashMap;

use cypher_parser::{
    ComparisonOperator, LiteralValue, ParseErrorCode, ParsedQuery, ProjectionItem, PropertyRef,
    QueryAst, QueryKind, parse_query,
};
use cypher_planner::{build_function_call_plan, build_logical_plan};
use function_registry::{FunctionRegistry, FunctionValue, ModelFunctionAdapter, RegistryError};
use graph_core::{
    Graph, Node, NodeInput, NodePatch, PropertyValue, Relationship, RelationshipInput,
    RelationshipPatch,
};
use thiserror::Error;
use tracing::{debug, instrument, trace, warn};

mod investigation_contracts;
mod investigation_response;

pub use investigation_contracts::*;
pub use investigation_response::*;

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution policy.
pub struct ExecutionPolicy {
    /// Read only by default.
    pub read_only_by_default: bool,
}

impl ExecutionPolicy {
    /// Strict default.
    pub fn strict_default() -> Self {
        Self {
            // Read only by default.
            read_only_by_default: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution status.
pub enum ExecutionStatus {
    /// Success.
    Success,
    /// Rejected.
    Rejected,
    /// Validation failed.
    ValidationFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution validation error.
pub struct ExecutionValidationError {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution fix hint.
pub struct ExecutionFixHint {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution record.
pub struct ExecutionRecord {
    /// Fields.
    pub fields: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution result data.
pub enum ExecutionResultData {
    /// Records.
    Records(Vec<ExecutionRecord>),
    /// Mutation summary — returned when a mutation has no RETURN clause.
    MutationSummary {
        /// Number of nodes created.
        nodes_created: usize,
        /// Number of relationships created.
        relationships_created: usize,
        /// Number of properties set.
        properties_set: usize,
        /// Number of nodes deleted (tombstoned).
        nodes_deleted: usize,
        /// Number of relationships deleted (tombstoned).
        relationships_deleted: usize,
    },
    /// Empty.
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Execution result.
pub struct ExecutionResult {
    /// Status.
    pub status: ExecutionStatus,
    /// Data.
    pub data: ExecutionResultData,
    /// Warnings.
    pub warnings: Vec<String>,
    /// Validation errors.
    pub validation_errors: Vec<ExecutionValidationError>,
    /// Fix hints.
    pub fix_hints: Vec<ExecutionFixHint>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
/// Execution error.
pub enum ExecutionError {
    #[error("invalid query: {0}")]
    /// Invalid query.
    InvalidQuery(String),
    #[error("function invocation failed: {0}")]
    /// Function invocation.
    FunctionInvocation(#[from] RegistryError),
}

#[derive(Clone, Debug)]
/// Cypher pipeline executor.
pub struct CypherPipelineExecutor {
    policy: ExecutionPolicy,
    graph: Graph,
}

impl CypherPipelineExecutor {
    /// Creates a new instance.
    pub fn new(policy: ExecutionPolicy) -> Self {
        Self {
            policy,
            // Graph.
            graph: Graph::new(),
        }
    }

    /// Sets the graph.
    pub fn with_graph(policy: ExecutionPolicy, graph: Graph) -> Self {
        Self { policy, graph }
    }

    /// Returns the underlying graph state.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Returns mutable access to the underlying graph state.
    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    /// Validates and plans a query without evaluating it against graph state.
    pub fn validate(&self, query_text: &str) -> Result<ExecutionResult, ExecutionError> {
        let _ast = parse_and_plan_query(query_text)?;

        Ok(ExecutionResult {
            status: ExecutionStatus::Success,
            data: ExecutionResultData::Empty,
            warnings: vec![],
            validation_errors: vec![],
            fix_hints: vec![],
        })
    }

    //
    // The executor keeps the parser->planner->execution flow explicit and
    // deterministic so gateway integration can rely on stable contracts.
    /// Execute.
    #[instrument(skip(self, query_text), fields(query_len = query_text.len()))]
    pub fn execute(&mut self, query_text: &str) -> Result<ExecutionResult, ExecutionError> {
        debug!(
            read_only_by_default = self.policy.read_only_by_default,
            "executing cypher query"
        );
        let ast = parse_and_plan_query(query_text)?;
        trace!(query_kind = ?ast.kind, "parsed cypher query");

        // Reject any query with write semantics under read-only policy.
        if self.policy.read_only_by_default
            && matches!(ast.kind, QueryKind::Mutation | QueryKind::Mixed)
        {
            warn!(query_kind = ?ast.kind, "query rejected by read-only execution policy");
            return Ok(ExecutionResult {
                status: ExecutionStatus::Rejected,
                data: ExecutionResultData::Empty,
                warnings: vec!["mutation execution is disabled in read-only policy".to_owned()],
                validation_errors: vec![ExecutionValidationError {
                    code: "WRITE_PERMISSION_REQUIRED".to_owned(),
                    message: "This pipeline is read-only by default".to_owned(),
                }],
                fix_hints: vec![ExecutionFixHint {
                    code: "ENABLE_MUTATION_MODE".to_owned(),
                    message:
                        "Enable explicit mutation permission or use validate-only mode for dry-run."
                            .to_owned(),
                }],
            });
        }

        // Execute based on query kind.
        match ast.kind {
            QueryKind::Read => {
                let records = match ast.query.as_ref() {
                    Some(query) => self.execute_structured_read_query(query)?,
                    None => Vec::new(),
                };
                debug!(
                    record_count = records.len(),
                    "query execution succeeded with read result"
                );
                Ok(ExecutionResult {
                    status: ExecutionStatus::Success,
                    data: ExecutionResultData::Records(records),
                    warnings: vec![],
                    validation_errors: vec![],
                    fix_hints: vec![],
                })
            }
            QueryKind::Mutation | QueryKind::Mixed => self.execute_mutation_query(&ast),
        }
    }

    // Function-call boundary intent:
    // Keep direct function invocation explicit through planner+registry so the
    // gateway can execute typed built-ins without bypassing plan contracts.
    /// Execute registered function.
    pub fn execute_registered_function(
        &self,
        registry: &FunctionRegistry,
        function_name: &str,
        args: &[FunctionValue],
        model_adapter: Option<&dyn ModelFunctionAdapter>,
    ) -> Result<FunctionValue, ExecutionError> {
        let _plan = build_function_call_plan(function_name);
        registry
            .invoke(function_name, args, model_adapter)
            .map_err(ExecutionError::FunctionInvocation)
    }

    /// Execute a mutation or mixed query.
    ///
    /// For mixed queries (MATCH + mutation), run the read portion first to
    /// produce bindings, then apply the mutation clauses against those bindings.
    /// For pure mutations (CREATE, MERGE without MATCH), execute directly.
    fn execute_mutation_query(
        &mut self,
        ast: &cypher_parser::QueryAst,
    ) -> Result<ExecutionResult, ExecutionError> {
        let query = match ast.query.as_ref() {
            Some(q) => q,
            None => {
                return Ok(ExecutionResult {
                    status: ExecutionStatus::Success,
                    data: ExecutionResultData::Empty,
                    warnings: vec![],
                    validation_errors: vec![],
                    fix_hints: vec![],
                });
            }
        };

        let mut nodes_created: usize = 0;
        let mut relationships_created: usize = 0;
        let mut properties_set: usize = 0;
        let mut nodes_deleted: usize = 0;
        let mut relationships_deleted: usize = 0;

        // Build rows from MATCH if present (for mixed queries).
        let mut rows: Vec<ExecutionRow> = if let Some(match_clause) = &query.match_clause {
            let mut matched = self.build_rows_from_match(match_clause)?;
            if let Some(where_clause) = &query.where_clause {
                matched.retain(|row| evaluate_where(row, where_clause));
            }
            matched
        } else {
            Vec::new()
        };

        // --- CREATE ---
        if let Some(create_clause) = &query.create_clause {
            if create_clause.relationship.is_some() {
                let (created_nodes, created_relationships) =
                    self.execute_create_relationship(create_clause, &mut rows)?;
                nodes_created += created_nodes;
                relationships_created += created_relationships;
            } else {
                for node_pattern in &create_clause.nodes {
                    let node = self.create_node_from_pattern(node_pattern, "CREATE")?;
                    nodes_created += 1;
                    let mut row = ExecutionRow::new();
                    row.bindings
                        .insert(node_pattern.variable.clone(), BindingValue::Node(node));
                    rows.push(row);
                }
            }
        }

        // --- MERGE ---
        if let Some(merge_clause) = &query.merge_clause {
            if merge_clause.relationship.is_none() {
                let pattern = &merge_clause.pattern;
                match self.find_matching_node(pattern)? {
                    Some(node) => {
                        let mut row = ExecutionRow::new();
                        row.bindings
                            .insert(pattern.variable.clone(), BindingValue::Node(node));
                        rows.push(row);
                    }
                    None => {
                        let node = self.create_node_from_pattern(pattern, "MERGE")?;
                        nodes_created += 1;
                        let mut row = ExecutionRow::new();
                        row.bindings
                            .insert(pattern.variable.clone(), BindingValue::Node(node));
                        rows.push(row);
                    }
                }
            }
            relationships_created += self.execute_merge_relationship(merge_clause, &mut rows)?;
        }

        // --- SET ---
        if let Some(set_clause) = &query.set_clause {
            let mut updated_rows = Vec::new();
            for row in &rows {
                for assignment in &set_clause.assignments {
                    let variable = &assignment.target.variable;
                    if let Some(BindingValue::Node(node)) = row.bindings.get(variable) {
                        let patch = NodePatch::default().set_property(
                            assignment.target.property.clone(),
                            literal_to_property_value(&assignment.value),
                        );
                        self.graph.update_node(node.id(), patch).map_err(|e| {
                            ExecutionError::InvalidQuery(format!("SET failed: {e}"))
                        })?;
                        properties_set += 1;
                    } else if let Some(BindingValue::Relationship(relationship)) =
                        row.bindings.get(variable)
                    {
                        let patch = RelationshipPatch::default().set_property(
                            assignment.target.property.clone(),
                            literal_to_property_value(&assignment.value),
                        );
                        self.graph
                            .update_relationship(relationship.id(), patch)
                            .map_err(|e| {
                                ExecutionError::InvalidQuery(format!("SET failed: {e}"))
                            })?;
                        properties_set += 1;
                    }
                }
                // Re-read the updated node for projection.
                let mut new_row = ExecutionRow::new();
                for (var, binding) in &row.bindings {
                    match binding {
                        BindingValue::Node(node) => {
                            if let Some(refreshed) =
                                self.graph.get_node(node.id()).map_err(|e| {
                                    ExecutionError::InvalidQuery(format!(
                                        "node re-read after SET failed: {e}"
                                    ))
                                })?
                            {
                                new_row
                                    .bindings
                                    .insert(var.clone(), BindingValue::Node(refreshed));
                            }
                        }
                        BindingValue::Relationship(rel) => {
                            if let Some(refreshed) =
                                self.graph.get_relationship(rel.id()).map_err(|e| {
                                    ExecutionError::InvalidQuery(format!(
                                        "relationship re-read after SET failed: {e}"
                                    ))
                                })?
                            {
                                new_row
                                    .bindings
                                    .insert(var.clone(), BindingValue::Relationship(refreshed));
                            }
                        }
                    }
                }
                updated_rows.push(new_row);
            }
            rows = updated_rows;
        }

        // --- REMOVE ---
        if let Some(remove_clause) = &query.remove_clause {
            let mut updated_rows = Vec::new();
            for row in &rows {
                for target in &remove_clause.targets {
                    if let Some(BindingValue::Node(node)) = row.bindings.get(&target.variable) {
                        let patch = NodePatch::default()
                            .set_property(target.property.clone(), PropertyValue::Null);
                        self.graph.update_node(node.id(), patch).map_err(|e| {
                            ExecutionError::InvalidQuery(format!("REMOVE failed: {e}"))
                        })?;
                        properties_set += 1;
                    }
                }
                // Re-read the updated node for projection.
                let mut new_row = ExecutionRow::new();
                for (var, binding) in &row.bindings {
                    match binding {
                        BindingValue::Node(node) => {
                            if let Some(refreshed) =
                                self.graph.get_node(node.id()).map_err(|e| {
                                    ExecutionError::InvalidQuery(format!(
                                        "node re-read after REMOVE failed: {e}"
                                    ))
                                })?
                            {
                                new_row
                                    .bindings
                                    .insert(var.clone(), BindingValue::Node(refreshed));
                            }
                        }
                        BindingValue::Relationship(rel) => {
                            new_row
                                .bindings
                                .insert(var.clone(), BindingValue::Relationship(rel.clone()));
                        }
                    }
                }
                updated_rows.push(new_row);
            }
            rows = updated_rows;
        }

        // --- DELETE ---
        if let Some(delete_clause) = &query.delete_clause {
            for row in &rows {
                for variable in &delete_clause.variables {
                    match row.bindings.get(variable) {
                        Some(BindingValue::Node(node)) => {
                            self.graph.tombstone_node(node.id()).map_err(|e| {
                                ExecutionError::InvalidQuery(format!("DELETE node failed: {e}"))
                            })?;
                            nodes_deleted += 1;
                        }
                        Some(BindingValue::Relationship(rel)) => {
                            self.graph.tombstone_relationship(rel.id()).map_err(|e| {
                                ExecutionError::InvalidQuery(format!(
                                    "DELETE relationship failed: {e}"
                                ))
                            })?;
                            relationships_deleted += 1;
                        }
                        None => {}
                    }
                }
            }
        }

        // Project results if RETURN is present, otherwise return mutation
        // summary.
        if let Some(return_clause) = &query.return_clause {
            let records = rows
                .into_iter()
                .map(|row| project_record(&row, &return_clause.items))
                .collect();
            Ok(ExecutionResult {
                status: ExecutionStatus::Success,
                data: ExecutionResultData::Records(records),
                warnings: vec![],
                validation_errors: vec![],
                fix_hints: vec![],
            })
        } else {
            Ok(ExecutionResult {
                status: ExecutionStatus::Success,
                data: ExecutionResultData::MutationSummary {
                    nodes_created,
                    relationships_created,
                    properties_set,
                    nodes_deleted,
                    relationships_deleted,
                },
                warnings: vec![],
                validation_errors: vec![],
                fix_hints: vec![],
            })
        }
    }

    // Create or reuse a MERGE relationship and bind it for SET/RETURN.
    fn execute_merge_relationship(
        &mut self,
        merge_clause: &cypher_parser::MergeClause,
        rows: &mut [ExecutionRow],
    ) -> Result<usize, ExecutionError> {
        let Some((relationship_pattern, target_pattern)) = &merge_clause.relationship else {
            return Ok(0);
        };
        let relationship_type = relationship_pattern.rel_type.as_deref().ok_or_else(|| {
            ExecutionError::InvalidQuery("MERGE relationship type is required".to_owned())
        })?;
        let target = match self.find_matching_node(target_pattern)? {
            Some(node) => node,
            None => self.create_node_from_pattern(target_pattern, "MERGE target")?,
        };
        let mut created = 0;
        for row in rows.iter_mut() {
            let source = match row.bindings.get(&merge_clause.pattern.variable) {
                Some(BindingValue::Node(node)) => node.clone(),
                _ => continue,
            };
            let existing = self
                .graph
                .relationships_between(source.id(), target.id())
                .map_err(|e| ExecutionError::InvalidQuery(format!("MERGE scan failed: {e}")))?
                .into_iter()
                .find(|relationship| relationship.rel_type().as_str() == relationship_type);
            let relationship = match existing {
                Some(relationship) => relationship,
                None => {
                    let relationship_id = self
                        .graph
                        .create_relationship(
                            RelationshipInput::new(
                                source.id().clone(),
                                relationship_type,
                                target.id().clone(),
                            )
                            .map_err(|e| {
                                ExecutionError::InvalidQuery(format!("MERGE create failed: {e}"))
                            })?,
                        )
                        .map_err(|e| {
                            ExecutionError::InvalidQuery(format!("MERGE create failed: {e}"))
                        })?;
                    created += 1;
                    self.graph
                        .get_relationship(&relationship_id)
                        .map_err(|e| {
                            ExecutionError::InvalidQuery(format!("MERGE lookup failed: {e}"))
                        })?
                        .expect("created relationship should be readable")
                }
            };
            row.bindings.insert(
                target_pattern.variable.clone(),
                BindingValue::Node(target.clone()),
            );
            if let Some(variable) = &relationship_pattern.variable {
                row.bindings
                    .insert(variable.clone(), BindingValue::Relationship(relationship));
            }
        }
        Ok(created)
    }

    fn execute_create_relationship(
        &mut self,
        create_clause: &cypher_parser::CreateClause,
        rows: &mut [ExecutionRow],
    ) -> Result<(usize, usize), ExecutionError> {
        let Some((relationship_pattern, target_pattern)) = &create_clause.relationship else {
            return Ok((0, 0));
        };
        let source_variable = &create_clause.nodes[0].variable;
        let relationship_type = relationship_pattern.rel_type.as_deref().ok_or_else(|| {
            ExecutionError::InvalidQuery("CREATE relationship type is required".to_owned())
        })?;
        let mut created = 0;
        for row in rows.iter_mut() {
            let source = match row.bindings.get(source_variable) {
                Some(BindingValue::Node(node)) => node.clone(),
                _ => continue,
            };
            let target = self.create_node_from_pattern(target_pattern, "CREATE target")?;
            let relationship_id = self
                .graph
                .create_relationship(
                    RelationshipInput::new(
                        source.id().clone(),
                        relationship_type,
                        target.id().clone(),
                    )
                    .map_err(|e| {
                        ExecutionError::InvalidQuery(format!("CREATE relationship failed: {e}"))
                    })?,
                )
                .map_err(|e| {
                    ExecutionError::InvalidQuery(format!("CREATE relationship failed: {e}"))
                })?;
            let relationship = self
                .graph
                .get_relationship(&relationship_id)
                .map_err(|e| {
                    ExecutionError::InvalidQuery(format!("CREATE relationship lookup failed: {e}"))
                })?
                .expect("created relationship should be readable");
            row.bindings
                .insert(target_pattern.variable.clone(), BindingValue::Node(target));
            if let Some(variable) = &relationship_pattern.variable {
                row.bindings
                    .insert(variable.clone(), BindingValue::Relationship(relationship));
            }
            created += 1;
        }
        Ok((created, created))
    }

    fn create_node_from_pattern(
        &mut self,
        pattern: &cypher_parser::NodePattern,
        operation: &str,
    ) -> Result<Node, ExecutionError> {
        let labels: Vec<&str> = pattern.label.as_deref().into_iter().collect();
        let mut input = NodeInput::new(labels);
        for (key, value) in &pattern.properties {
            input = input.with_property(key.clone(), literal_to_property_value(value));
        }
        let node_id = self.graph.create_node(input).map_err(|e| {
            ExecutionError::InvalidQuery(format!("{operation} node creation failed: {e}"))
        })?;
        self.graph
            .get_node(&node_id)
            .map_err(|e| ExecutionError::InvalidQuery(format!("{operation} lookup failed: {e}")))?
            .ok_or_else(|| {
                ExecutionError::InvalidQuery(format!("{operation} created node is unavailable"))
            })
    }

    /// Find an existing node matching a node pattern's label and inline
    /// properties (used by MERGE).
    fn find_matching_node(
        &self,
        pattern: &cypher_parser::NodePattern,
    ) -> Result<Option<Node>, ExecutionError> {
        let nodes = self
            .graph
            .list_nodes()
            .map_err(|e| ExecutionError::InvalidQuery(format!("MERGE node scan failed: {e}")))?;

        for node in nodes {
            // Check label match.
            if let Some(label) = &pattern.label
                && !node.has_label(label)
            {
                continue;
            }
            // Check all inline property matches.
            let all_match = pattern.properties.iter().all(|(key, expected)| {
                let expected_pv = literal_to_property_value(expected);
                node.property(key)
                    .map(|actual| *actual == expected_pv)
                    .unwrap_or(false)
            });
            if all_match {
                return Ok(Some(node));
            }
        }

        Ok(None)
    }

    fn execute_structured_read_query(
        &self,
        query: &ParsedQuery,
    ) -> Result<Vec<ExecutionRecord>, ExecutionError> {
        let match_clause = match &query.match_clause {
            Some(clause) => clause,
            None => return Ok(Vec::new()),
        };

        let mut rows = self.build_rows_from_match(match_clause)?;

        if let Some(where_clause) = &query.where_clause {
            rows.retain(|row| evaluate_where(row, where_clause));
        }

        let Some(return_clause) = query.return_clause.as_ref() else {
            return Ok(Vec::new());
        };

        if let Some(order_by) = &return_clause.order_by {
            let property = order_by.field.clone();
            rows.sort_by(|left, right| {
                let left_value = property_ref_to_sort_key(left, &property);
                let right_value = property_ref_to_sort_key(right, &property);
                let ordering = left_value.cmp(&right_value);
                match order_by.direction {
                    cypher_parser::OrderDirection::Asc => ordering,
                    cypher_parser::OrderDirection::Desc => ordering.reverse(),
                }
            });
        }

        if let Some(skip) = return_clause.skip {
            rows = rows.into_iter().skip(skip).collect();
        }

        if let Some(limit) = return_clause.limit {
            rows.truncate(limit);
        }

        let mut records = if return_clause
            .items
            .iter()
            .all(|item| matches!(item, ProjectionItem::Count(_)))
        {
            let mut fields = HashMap::new();
            fields.insert("count".to_owned(), rows.len().to_string());
            vec![ExecutionRecord { fields }]
        } else {
            rows.into_iter()
                .map(|row| project_record(&row, &return_clause.items))
                .collect()
        };

        if return_clause.distinct {
            let mut seen = std::collections::HashSet::new();
            records.retain(|record| {
                let mut keys = record.fields.iter().collect::<Vec<(&String, &String)>>();
                keys.sort_by(|left, right| left.0.cmp(right.0));
                let signature = keys
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<String>>()
                    .join("|");
                seen.insert(signature)
            });
        }

        Ok(records)
    }

    fn build_rows_from_match(
        &self,
        match_clause: &cypher_parser::MatchClause,
    ) -> Result<Vec<ExecutionRow>, ExecutionError> {
        if let Some((relationship_pattern, end_pattern)) = &match_clause.relationship {
            let relationships = self.graph.list_relationships().map_err(|error| {
                ExecutionError::InvalidQuery(format!("graph traversal failed: {error}"))
            })?;
            let mut rows = Vec::new();

            for relationship in relationships {
                if !relationship_type_matches(
                    &relationship,
                    relationship_pattern.rel_type.as_deref(),
                ) {
                    continue;
                }

                let Some(source) = self
                    .graph
                    .get_node(relationship.source())
                    .map_err(|error| {
                        ExecutionError::InvalidQuery(format!("graph node lookup failed: {error}"))
                    })?
                else {
                    continue;
                };

                let Some(target) = self
                    .graph
                    .get_node(relationship.target())
                    .map_err(|error| {
                        ExecutionError::InvalidQuery(format!("graph node lookup failed: {error}"))
                    })?
                else {
                    continue;
                };

                if !node_pattern_matches(&source, &match_clause.start) {
                    continue;
                }
                if !node_pattern_matches(&target, end_pattern) {
                    continue;
                }

                let mut row = ExecutionRow::new();
                row.bindings.insert(
                    match_clause.start.variable.clone(),
                    BindingValue::Node(source),
                );
                row.bindings
                    .insert(end_pattern.variable.clone(), BindingValue::Node(target));
                if let Some(variable) = &relationship_pattern.variable {
                    row.bindings
                        .insert(variable.clone(), BindingValue::Relationship(relationship));
                }
                rows.push(row);
            }

            return Ok(rows);
        }

        let nodes = self.graph.list_nodes().map_err(|error| {
            ExecutionError::InvalidQuery(format!("graph traversal failed: {error}"))
        })?;
        let rows = nodes
            .into_iter()
            .filter(|node| node_pattern_matches(node, &match_clause.start))
            .map(|node| {
                let mut row = ExecutionRow::new();
                row.bindings.insert(
                    match_clause.start.variable.clone(),
                    BindingValue::Node(node),
                );
                row
            })
            .collect::<Vec<ExecutionRow>>();

        Ok(rows)
    }
}

#[derive(Clone, Debug)]
struct ExecutionRow {
    bindings: HashMap<String, BindingValue>,
}

impl ExecutionRow {
    fn new() -> Self {
        Self {
            // Bindings.
            bindings: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
enum BindingValue {
    /// Node.
    Node(Node),
    /// Relationship.
    Relationship(Relationship),
}

fn node_label_matches(node: &Node, expected_label: Option<&str>) -> bool {
    expected_label
        .map(|label| node.has_label(label))
        .unwrap_or(true)
}

fn node_pattern_matches(node: &Node, pattern: &cypher_parser::NodePattern) -> bool {
    node_label_matches(node, pattern.label.as_deref())
        && pattern.properties.iter().all(|(key, expected)| {
            node.property(key)
                .is_some_and(|actual| *actual == literal_to_property_value(expected))
        })
}

fn relationship_type_matches(relationship: &Relationship, expected_type: Option<&str>) -> bool {
    expected_type
        .map(|rel_type| relationship.rel_type().as_str() == rel_type)
        .unwrap_or(true)
}

fn evaluate_where(row: &ExecutionRow, where_clause: &cypher_parser::WhereClause) -> bool {
    let Some(left_value) = property_ref_value(row, &where_clause.left) else {
        return false;
    };

    match (&left_value, &where_clause.right) {
        (PropertyValue::String(left), LiteralValue::String(right)) => match where_clause.operator {
            ComparisonOperator::Eq => left == right,
            ComparisonOperator::NotEq => left != right,
            _ => false,
        },
        (PropertyValue::Integer(left), LiteralValue::Integer(right)) => match where_clause.operator
        {
            ComparisonOperator::Eq => left == right,
            ComparisonOperator::NotEq => left != right,
            ComparisonOperator::Gt => left > right,
            ComparisonOperator::Gte => left >= right,
            ComparisonOperator::Lt => left < right,
            ComparisonOperator::Lte => left <= right,
        },
        (PropertyValue::Bool(left), LiteralValue::Boolean(right)) => match where_clause.operator {
            ComparisonOperator::Eq => left == right,
            ComparisonOperator::NotEq => left != right,
            _ => false,
        },
        _ => false,
    }
}

fn property_ref_to_sort_key(row: &ExecutionRow, property: &PropertyRef) -> String {
    property_ref_value(row, property)
        .map(property_value_to_string)
        .unwrap_or_default()
}

fn project_record(row: &ExecutionRow, items: &[ProjectionItem]) -> ExecutionRecord {
    let mut fields = HashMap::new();

    for item in items {
        match item {
            ProjectionItem::Variable(variable) => {
                if let Some(value) = row.bindings.get(variable) {
                    fields.insert(variable.clone(), binding_to_string(value));
                }
            }
            ProjectionItem::Property(property_ref) => {
                if let Some(value) = property_ref_value(row, property_ref) {
                    fields.insert(
                        format!("{}.{}", property_ref.variable, property_ref.property),
                        property_value_to_string(value),
                    );
                }
            }
            ProjectionItem::Count(variable) => {
                if row.bindings.contains_key(variable) {
                    fields.insert("count".to_owned(), "1".to_owned());
                }
            }
        }
    }

    ExecutionRecord { fields }
}

fn binding_to_string(value: &BindingValue) -> String {
    match value {
        BindingValue::Node(node) => node.id().as_str().to_owned(),
        BindingValue::Relationship(relationship) => relationship.id().as_str().to_owned(),
    }
}

fn property_ref_value<'a>(
    row: &'a ExecutionRow,
    property_ref: &PropertyRef,
) -> Option<&'a PropertyValue> {
    match row.bindings.get(&property_ref.variable) {
        Some(BindingValue::Node(node)) => node.property(&property_ref.property),
        Some(BindingValue::Relationship(relationship)) => {
            relationship.property(&property_ref.property)
        }
        None => None,
    }
}

fn property_value_to_string(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_owned(),
        PropertyValue::Bool(v) => v.to_string(),
        PropertyValue::Integer(v) => v.to_string(),
        PropertyValue::Float(v) => v.to_string(),
        PropertyValue::String(v) => v.clone(),
        PropertyValue::StringList(values) => values.join(","),
        PropertyValue::IntegerList(values) => values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<String>>()
            .join(","),
        PropertyValue::FloatList(values) => values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<String>>()
            .join(","),
        PropertyValue::BoolList(values) => values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<String>>()
            .join(","),
    }
}

/// Convert a parser `LiteralValue` into a graph-core `PropertyValue`.
fn literal_to_property_value(literal: &LiteralValue) -> PropertyValue {
    match literal {
        LiteralValue::String(s) => PropertyValue::String(s.clone()),
        LiteralValue::Integer(i) => PropertyValue::Integer(*i),
        LiteralValue::Boolean(b) => PropertyValue::Bool(*b),
    }
}

fn parse_and_plan_query(query_text: &str) -> Result<QueryAst, ExecutionError> {
    let ast = parse_query(query_text).map_err(|parse_error| match parse_error.code {
        ParseErrorCode::EmptyQuery => {
            ExecutionError::InvalidQuery("query text must not be empty".to_owned())
        }
        ParseErrorCode::UnsupportedFeature | ParseErrorCode::InvalidSyntax => {
            ExecutionError::InvalidQuery(parse_error.message)
        }
    })?;
    let _plan = build_logical_plan(&ast);
    Ok(ast)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cypher_parser::{
        ComparisonOperator, LiteralValue, MatchClause, NodePattern, ProjectionItem, PropertyRef,
        ReturnClause, WhereClause,
    };
    use graph_core::{Graph, NodeInput, PropertyValue, RecordStatus, RelationshipInput};

    fn small_graph() -> Graph {
        let mut graph = Graph::new();
        let source = graph
            .create_node(
                NodeInput::new(["Actor"])
                    .with_status(RecordStatus::Exportable)
                    .with_property("name", PropertyValue::String("alpha".to_owned()))
                    .with_property("score", PropertyValue::Integer(10))
                    .with_property("active", PropertyValue::Bool(true)),
            )
            .expect("source node should be created");
        let target = graph
            .create_node(
                NodeInput::new(["Narrative"])
                    .with_status(RecordStatus::Exportable)
                    .with_property("name", PropertyValue::String("n1".to_owned()))
                    .with_property("score", PropertyValue::Integer(20))
                    .with_property("active", PropertyValue::Bool(false)),
            )
            .expect("target node should be created");

        graph
            .create_relationship(
                RelationshipInput::new(source, "AMPLIFIES", target)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable)
                    .with_property("enabled", PropertyValue::Bool(true)),
            )
            .expect("relationship should be created");

        graph
    }

    #[test]
    fn helper_label_and_relationship_type_matching_cover_true_and_false_paths() {
        let graph = small_graph();
        let actor = graph
            .list_nodes()
            .expect("nodes should list")
            .into_iter()
            .find(|node| node.has_label("Actor"))
            .expect("actor node should exist");
        let relationship = graph
            .list_relationships()
            .expect("relationships should list")
            .into_iter()
            .next()
            .expect("relationship should exist");

        assert!(node_label_matches(&actor, Some("Actor")));
        assert!(!node_label_matches(&actor, Some("Indicator")));
        assert!(node_label_matches(&actor, None));

        assert!(relationship_type_matches(&relationship, Some("AMPLIFIES")));
        assert!(!relationship_type_matches(
            &relationship,
            Some("RELATED_TO")
        ));
        assert!(relationship_type_matches(&relationship, None));
    }

    #[test]
    fn evaluate_where_handles_integer_bool_and_missing_property_paths() {
        let graph = small_graph();
        let actor = graph
            .list_nodes()
            .expect("nodes should list")
            .into_iter()
            .find(|node| node.has_label("Actor"))
            .expect("actor node should exist");
        let mut row = ExecutionRow::new();
        row.bindings
            .insert("n".to_owned(), BindingValue::Node(actor));

        let integer_gte = WhereClause {
            left: PropertyRef {
                variable: "n".to_owned(),
                property: "score".to_owned(),
            },
            operator: ComparisonOperator::Gte,
            right: LiteralValue::Integer(10),
        };
        assert!(evaluate_where(&row, &integer_gte));

        let bool_not_eq = WhereClause {
            left: PropertyRef {
                variable: "n".to_owned(),
                property: "active".to_owned(),
            },
            operator: ComparisonOperator::NotEq,
            right: LiteralValue::Boolean(false),
        };
        assert!(evaluate_where(&row, &bool_not_eq));

        let missing_property = WhereClause {
            left: PropertyRef {
                variable: "n".to_owned(),
                property: "missing".to_owned(),
            },
            operator: ComparisonOperator::Eq,
            right: LiteralValue::String("x".to_owned()),
        };
        assert!(!evaluate_where(&row, &missing_property));
    }

    #[test]
    fn project_record_handles_variable_property_and_count_paths() {
        let graph = small_graph();
        let actor = graph
            .list_nodes()
            .expect("nodes should list")
            .into_iter()
            .find(|node| node.has_label("Actor"))
            .expect("actor node should exist");
        let mut row = ExecutionRow::new();
        row.bindings
            .insert("n".to_owned(), BindingValue::Node(actor));

        let record = project_record(
            &row,
            &[
                ProjectionItem::Variable("n".to_owned()),
                ProjectionItem::Property(PropertyRef {
                    variable: "n".to_owned(),
                    property: "name".to_owned(),
                }),
                ProjectionItem::Count("n".to_owned()),
                ProjectionItem::Count("missing".to_owned()),
            ],
        );

        assert!(record.fields.contains_key("n"));
        assert_eq!(record.fields.get("n.name"), Some(&"alpha".to_owned()));
        assert_eq!(record.fields.get("count"), Some(&"1".to_owned()));
    }

    #[test]
    fn execute_structured_read_query_returns_empty_without_match_or_return_clause() {
        let executor =
            CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), small_graph());

        let no_match = ParsedQuery {
            match_clause: None,
            where_clause: None,
            return_clause: Some(ReturnClause {
                distinct: false,
                items: vec![ProjectionItem::Variable("n".to_owned())],
                order_by: None,
                skip: None,
                limit: None,
            }),
            create_clause: None,
            merge_clause: None,
            set_clause: None,
            delete_clause: None,
            remove_clause: None,
        };
        let no_return = ParsedQuery {
            match_clause: Some(MatchClause {
                optional: false,
                start: NodePattern {
                    variable: "n".to_owned(),
                    label: Some("Actor".to_owned()),
                    properties: Vec::new(),
                },
                relationship: None,
            }),
            where_clause: None,
            return_clause: None,
            create_clause: None,
            merge_clause: None,
            set_clause: None,
            delete_clause: None,
            remove_clause: None,
        };

        assert!(
            executor
                .execute_structured_read_query(&no_match)
                .expect("no match should return empty vector")
                .is_empty()
        );
        assert!(
            executor
                .execute_structured_read_query(&no_return)
                .expect("no return should return empty vector")
                .is_empty()
        );
    }

    #[test]
    fn property_value_to_string_formats_all_supported_variants() {
        assert_eq!(property_value_to_string(&PropertyValue::Null), "null");
        assert_eq!(property_value_to_string(&PropertyValue::Bool(true)), "true");
        assert_eq!(property_value_to_string(&PropertyValue::Integer(7)), "7");
        assert_eq!(property_value_to_string(&PropertyValue::Float(1.5)), "1.5");
        assert_eq!(
            property_value_to_string(&PropertyValue::String("x".to_owned())),
            "x"
        );
        assert_eq!(
            property_value_to_string(&PropertyValue::StringList(vec![
                "a".to_owned(),
                "b".to_owned(),
            ])),
            "a,b"
        );
        assert_eq!(
            property_value_to_string(&PropertyValue::IntegerList(vec![1, 2])),
            "1,2"
        );
        assert_eq!(
            property_value_to_string(&PropertyValue::FloatList(vec![1.0, 2.0])),
            "1,2"
        );
        assert_eq!(
            property_value_to_string(&PropertyValue::BoolList(vec![true, false])),
            "true,false"
        );
    }
}
