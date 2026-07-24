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

//! Cypher query parser for the intelligence graph engine.
//!
//! Parses a subset of openCypher syntax into a structured AST suitable for
//! logical planning and execution. Covers `MATCH`, `WHERE`, `RETURN`, `CREATE`,
//! `MERGE`, `SET`, `REMOVE`, `DELETE`, aggregation functions, and ordering.

use thiserror::Error;

mod investigation_query;

pub use investigation_query::*;

#[derive(Clone, Debug, PartialEq, Eq)]
/// Query kind.
pub enum QueryKind {
    /// Read.
    Read,
    /// Mutation.
    Mutation,
    /// Mixed.
    Mixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Clause kind.
pub enum ClauseKind {
    /// Match.
    Match,
    /// Optional match.
    OptionalMatch,
    /// Where.
    Where,
    /// Return.
    Return,
    /// With.
    With,
    /// Distinct.
    Distinct,
    /// Order by.
    OrderBy,
    /// Skip.
    Skip,
    /// Limit.
    Limit,
    /// Create.
    Create,
    /// Merge.
    Merge,
    /// Set.
    Set,
    /// Remove.
    Remove,
    /// Delete.
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Aggregation function.
pub enum AggregationFunction {
    /// Count.
    Count,
    /// Sum.
    Sum,
    /// Avg.
    Avg,
    /// Min.
    Min,
    /// Max.
    Max,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Query ast.
pub struct QueryAst {
    /// Normalized query.
    pub normalized_query: String,
    /// Kind.
    pub kind: QueryKind,
    /// Clauses.
    pub clauses: Vec<ClauseKind>,
    /// Aggregations.
    pub aggregations: Vec<AggregationFunction>,
    /// Query.
    pub query: Option<ParsedQuery>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Parsed query.
pub struct ParsedQuery {
    /// Match clause.
    pub match_clause: Option<MatchClause>,
    /// Where clause.
    pub where_clause: Option<WhereClause>,
    /// Return clause.
    pub return_clause: Option<ReturnClause>,
    /// Create clause.
    pub create_clause: Option<CreateClause>,
    /// Merge clause.
    pub merge_clause: Option<MergeClause>,
    /// Set clause.
    pub set_clause: Option<SetClause>,
    /// Delete clause.
    pub delete_clause: Option<DeleteClause>,
    /// Remove clause.
    pub remove_clause: Option<RemoveClause>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Create clause — carries node patterns to insert into the graph.
pub struct CreateClause {
    /// Node patterns to create.
    pub nodes: Vec<NodePattern>,
    /// Optional relationship pattern following the source node.
    pub relationship: Option<(RelationshipPattern, NodePattern)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Merge clause — find-or-create semantics for a single node pattern.
pub struct MergeClause {
    /// Node pattern to match or create.
    pub pattern: NodePattern,
    /// Optional relationship pattern (source pattern is bound from MATCH or
    /// earlier clause; target pattern follows the arrow).
    pub relationship: Option<(RelationshipPattern, NodePattern)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Set clause — property assignments on matched bindings.
pub struct SetClause {
    /// Property assignments.
    pub assignments: Vec<SetAssignment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A single SET assignment: `variable.property = value`.
pub struct SetAssignment {
    /// Target property reference.
    pub target: PropertyRef,
    /// Value to assign.
    pub value: LiteralValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Delete clause — variables to tombstone.
pub struct DeleteClause {
    /// Variable names bound in MATCH that should be tombstoned.
    pub variables: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Remove clause — property references to nullify.
pub struct RemoveClause {
    /// Property references to remove (set to null).
    pub targets: Vec<PropertyRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Match clause.
pub struct MatchClause {
    /// Optional.
    pub optional: bool,
    /// Start.
    pub start: NodePattern,
    /// Relationship.
    pub relationship: Option<(RelationshipPattern, NodePattern)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Node pattern.
pub struct NodePattern {
    /// Variable.
    pub variable: String,
    /// Label.
    pub label: Option<String>,
    /// Inline properties from `{key: value, ...}` syntax.
    pub properties: Vec<(String, LiteralValue)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Relationship pattern.
pub struct RelationshipPattern {
    /// Variable.
    pub variable: Option<String>,
    /// Rel type.
    pub rel_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Where clause.
pub struct WhereClause {
    /// Left.
    pub left: PropertyRef,
    /// Operator.
    pub operator: ComparisonOperator,
    /// Right.
    pub right: LiteralValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Property ref.
pub struct PropertyRef {
    /// Variable.
    pub variable: String,
    /// Property.
    pub property: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Comparison operator.
pub enum ComparisonOperator {
    /// Eq.
    Eq,
    /// Not eq.
    NotEq,
    /// Gt.
    Gt,
    /// Gte.
    Gte,
    /// Lt.
    Lt,
    /// Lte.
    Lte,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Literal value.
pub enum LiteralValue {
    /// String.
    String(String),
    /// Integer.
    Integer(i64),
    /// Boolean.
    Boolean(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Return clause.
pub struct ReturnClause {
    /// Distinct.
    pub distinct: bool,
    /// Items.
    pub items: Vec<ProjectionItem>,
    /// Order by.
    pub order_by: Option<OrderBy>,
    /// Skip.
    pub skip: Option<usize>,
    /// Limit.
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Projection item.
pub enum ProjectionItem {
    /// Variable.
    Variable(String),
    /// Property.
    Property(PropertyRef),
    /// Count.
    Count(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Order by.
pub struct OrderBy {
    /// Field.
    pub field: PropertyRef,
    /// Direction.
    pub direction: OrderDirection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Order direction.
pub enum OrderDirection {
    /// Asc.
    Asc,
    /// Desc.
    Desc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Parse error code.
pub enum ParseErrorCode {
    /// Empty query.
    EmptyQuery,
    /// Unsupported feature.
    UnsupportedFeature,
    /// Invalid syntax.
    InvalidSyntax,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
/// Parse error.
pub struct ParseError {
    /// Code.
    pub code: ParseErrorCode,
    /// Message.
    pub message: String,
    /// Suggestion.
    pub suggestion: Option<String>,
}

/// Mvp supported clauses.
pub fn mvp_supported_clauses() -> Vec<&'static str> {
    vec![
        "MATCH",
        "OPTIONAL MATCH",
        "WHERE",
        "RETURN",
        "WITH",
        "ORDER BY",
        "SKIP",
        "LIMIT",
        "DISTINCT",
        "COUNT",
        "SUM",
        "AVG",
        "MIN",
        "MAX",
        "CREATE",
        "MERGE",
        "SET",
        "REMOVE",
        "DELETE",
    ]
}

//
// This parser establishes a deterministic AST contract for the MVP pipeline.
// It intentionally performs lightweight normalization and feature gating first.
/// Parse query.
pub fn parse_query(query_text: &str) -> Result<QueryAst, ParseError> {
    let normalized_query = query_text.split_whitespace().collect::<Vec<_>>().join(" ");

    if normalized_query.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::EmptyQuery,
            message: "query text must not be empty".to_owned(),
            suggestion: Some("Provide a Cypher query with at least one clause.".to_owned()),
        });
    }

    if let Some(feature) = detect_unsupported_feature(&normalized_query) {
        return Err(ParseError {
            code: ParseErrorCode::UnsupportedFeature,
            message: format!("unsupported Cypher feature: {feature}"),
            suggestion: Some(
                "Rewrite the query with the supported openCypher-compatible MVP subset.".to_owned(),
            ),
        });
    }

    let clauses = extract_supported_clauses(&normalized_query);
    if clauses.is_empty() {
        return Err(ParseError {
 code: ParseErrorCode::InvalidSyntax,
 message: "query does not contain a supported MVP clause".to_owned(),
 suggestion: Some(
 "Start the query with MATCH/OPTIONAL MATCH (read) or CREATE/MERGE/SET/REMOVE/DELETE (mutation)."
 .to_owned(),
 ),
 });
    }

    let kind = classify_query_kind(&clauses);
    let query = parse_structured_query(&normalized_query, &clauses).ok();
    let aggregations = extract_basic_aggregations(&query, &normalized_query);

    Ok(QueryAst {
        kind,
        normalized_query,
        clauses,
        aggregations,
        query,
    })
}

fn classify_query_kind(clauses: &[ClauseKind]) -> QueryKind {
    let has_read = clauses.iter().any(|clause| {
        matches!(
            clause,
            ClauseKind::Match
                | ClauseKind::OptionalMatch
                | ClauseKind::Where
                | ClauseKind::Return
                | ClauseKind::With
                | ClauseKind::OrderBy
                | ClauseKind::Skip
                | ClauseKind::Limit
        )
    });
    let has_write = clauses.iter().any(|clause| {
        matches!(
            clause,
            ClauseKind::Create
                | ClauseKind::Merge
                | ClauseKind::Set
                | ClauseKind::Remove
                | ClauseKind::Delete
        )
    });

    match (has_read, has_write) {
        (true, true) => QueryKind::Mixed,
        (_, true) => QueryKind::Mutation,
        _ => QueryKind::Read,
    }
}

fn detect_unsupported_feature(query_text: &str) -> Option<&'static str> {
    let upper = query_text.to_ascii_uppercase();
    let tokens = upper
        .split_whitespace()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<&str>>();

    for window in tokens.windows(2) {
        if window == ["LOAD", "CSV"] {
            return Some("LOAD CSV");
        }
        if window == ["CALL", "APOC"] {
            return Some("CALL APOC");
        }
        if window == ["CALL", "DBMS"] {
            return Some("CALL DBMS");
        }
        if window == ["DETACH", "DELETE"] {
            return Some("DETACH DELETE");
        }
    }

    if tokens.contains(&"UNWIND") {
        return Some("UNWIND");
    }

    if tokens.contains(&"FOREACH") {
        return Some("FOREACH");
    }

    if upper.contains("CALL {") {
        return Some("CALL {");
    }

    None
}

fn extract_supported_clauses(query_text: &str) -> Vec<ClauseKind> {
    let mut clauses = Vec::new();
    let mut index = 0;
    let tokens = query_text
        .split_whitespace()
        .map(|token| token.to_ascii_uppercase())
        .collect::<Vec<String>>();

    while index < tokens.len() {
        let current = tokens[index].as_str();
        let next = tokens.get(index + 1).map(String::as_str);

        if current == "OPTIONAL" && next == Some("MATCH") {
            clauses.push(ClauseKind::OptionalMatch);
            index += 2;
            continue;
        }

        if current == "ORDER" && next == Some("BY") {
            clauses.push(ClauseKind::OrderBy);
            index += 2;
            continue;
        }

        if current == "MATCH" {
            clauses.push(ClauseKind::Match);
            index += 1;
            continue;
        }
        if current == "WHERE" {
            clauses.push(ClauseKind::Where);
            index += 1;
            continue;
        }
        if current == "RETURN" {
            clauses.push(ClauseKind::Return);
            index += 1;
            continue;
        }
        if current == "WITH" {
            clauses.push(ClauseKind::With);
            index += 1;
            continue;
        }
        if current == "SKIP" {
            clauses.push(ClauseKind::Skip);
            index += 1;
            continue;
        }
        if current == "LIMIT" {
            clauses.push(ClauseKind::Limit);
            index += 1;
            continue;
        }
        if current == "DISTINCT" {
            clauses.push(ClauseKind::Distinct);
            index += 1;
            continue;
        }
        if current == "CREATE" {
            clauses.push(ClauseKind::Create);
            index += 1;
            continue;
        }
        if current == "MERGE" {
            clauses.push(ClauseKind::Merge);
            index += 1;
            continue;
        }
        if current == "SET" {
            clauses.push(ClauseKind::Set);
            index += 1;
            continue;
        }
        if current == "REMOVE" {
            clauses.push(ClauseKind::Remove);
            index += 1;
            continue;
        }
        if current == "DELETE" {
            clauses.push(ClauseKind::Delete);
            index += 1;
            continue;
        }

        index += 1;
    }

    clauses
}

fn parse_structured_query(
    query_text: &str,
    clauses: &[ClauseKind],
) -> Result<ParsedQuery, ParseError> {
    let has_match = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Match | ClauseKind::OptionalMatch));
    let has_return = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Return));
    let has_create = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Create));
    let has_merge = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Merge));
    let has_set = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Set));
    let has_delete = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Delete));
    let has_remove = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Remove));
    let has_mutation = has_create || has_merge || has_set || has_delete || has_remove;
    let has_with = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::With));

    // Pure mutation queries: CREATE/MERGE without MATCH are valid.
    // Mixed queries: MATCH + mutation clause(s) are valid.
    // Read-only queries: MATCH + RETURN (existing path).

    if has_with {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "query shape is outside structured MVP parser path".to_owned(),
            suggestion: None,
        });
    }

    if has_mutation {
        return parse_mutation_query(query_text, clauses);
    }

    if !has_match || !has_return {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "query shape is outside structured MVP parser path".to_owned(),
            suggestion: None,
        });
    }

    parse_match_where_return_query(query_text)
}

/// Parse a mutation or mixed query into a `ParsedQuery` with mutation clause
/// fields populated.
///
/// Supported shapes:
/// - `CREATE (n:Label {props}) [RETURN ...]`
/// - `MERGE (n:Label {props}) [RETURN ...]`
/// - `MATCH ... [WHERE ...] SET ... [RETURN ...]`
/// - `MATCH ... [WHERE ...] DELETE ...`
/// - `MATCH ... [WHERE ...] REMOVE ... [RETURN ...]`
fn parse_mutation_query(
    query_text: &str,
    clauses: &[ClauseKind],
) -> Result<ParsedQuery, ParseError> {
    let upper = query_text.to_ascii_uppercase();
    let has_match = clauses
        .iter()
        .any(|clause| matches!(clause, ClauseKind::Match | ClauseKind::OptionalMatch));

    let mut parsed = ParsedQuery {
        match_clause: None,
        where_clause: None,
        return_clause: None,
        create_clause: None,
        merge_clause: None,
        set_clause: None,
        delete_clause: None,
        remove_clause: None,
    };

    // Determine clause boundaries by keyword positions.
    let create_pos = find_clause_keyword(&upper, "CREATE");
    let merge_pos = find_clause_keyword(&upper, "MERGE");
    let set_pos = find_clause_keyword(&upper, "SET");
    let delete_pos = find_clause_keyword(&upper, "DELETE");
    let remove_pos = find_clause_keyword(&upper, "REMOVE");
    let return_pos = find_clause_keyword(&upper, "RETURN");

    // For mixed queries, parse the MATCH+WHERE portion first.
    if has_match {
        // Find the end of the MATCH+WHERE section — it ends at the first
        // mutation keyword or at RETURN.
        let match_end = [
            create_pos, merge_pos, set_pos, delete_pos, remove_pos, return_pos,
        ]
        .into_iter()
        .flatten()
        .min();
        let match_section = if let Some(end) = match_end {
            &query_text[..end]
        } else {
            query_text
        };
        let match_result = parse_match_where_return_only_match(match_section)?;
        parsed.match_clause = match_result.0;
        parsed.where_clause = match_result.1;
    }

    // Parse CREATE clause.
    if let Some(pos) = create_pos {
        let create_start = pos + "CREATE".len();
        let create_end = [set_pos, return_pos, delete_pos, remove_pos]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(query_text.len());
        let create_body = query_text[create_start..create_end].trim();
        parsed.create_clause = Some(parse_create_clause(create_body)?);
    }

    // Parse MERGE clause.
    if let Some(pos) = merge_pos {
        let merge_start = pos + "MERGE".len();
        let merge_end = [set_pos, return_pos, delete_pos, remove_pos]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(query_text.len());
        let merge_body = query_text[merge_start..merge_end].trim();
        parsed.merge_clause = Some(parse_merge_clause(merge_body)?);
    }

    // Parse SET clause.
    if let Some(pos) = set_pos {
        let set_start = pos + "SET".len();
        let set_end = [return_pos, delete_pos, remove_pos]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(query_text.len());
        let set_body = query_text[set_start..set_end].trim();
        parsed.set_clause = Some(parse_set_clause(set_body)?);
    }

    // Parse DELETE clause.
    if let Some(pos) = delete_pos {
        let delete_start = pos + "DELETE".len();
        let delete_end = return_pos.unwrap_or(query_text.len());
        let delete_body = query_text[delete_start..delete_end].trim();
        parsed.delete_clause = Some(parse_delete_clause(delete_body)?);
    }

    // Parse REMOVE clause.
    if let Some(pos) = remove_pos {
        let remove_start = pos + "REMOVE".len();
        let remove_end = return_pos.unwrap_or(query_text.len());
        let remove_body = query_text[remove_start..remove_end].trim();
        parsed.remove_clause = Some(parse_remove_clause(remove_body)?);
    }

    // Parse RETURN clause.
    if let Some(pos) = return_pos {
        let return_body = &query_text[pos + "RETURN".len()..];
        parsed.return_clause = Some(parse_return_clause(return_body.trim())?);
    }

    Ok(parsed)
}

/// Parse only the MATCH [WHERE] portion (no RETURN required).
/// Returns the match clause and optional where clause.
fn parse_match_where_return_only_match(
    query_text: &str,
) -> Result<(Option<MatchClause>, Option<WhereClause>), ParseError> {
    let upper = query_text.to_ascii_uppercase();
    let optional_prefix = "OPTIONAL MATCH ";
    let match_prefix = "MATCH ";

    let (optional, body) = if upper.starts_with(optional_prefix) {
        (true, &query_text[optional_prefix.len()..])
    } else if upper.starts_with(match_prefix) {
        (false, &query_text[match_prefix.len()..])
    } else {
        return Ok((None, None));
    };

    let body_upper = body.to_ascii_uppercase();
    let where_pos = find_clause_keyword(&body_upper, "WHERE");

    let (match_part, where_part) = if let Some(where_index) = where_pos {
        (
            body[..where_index].trim(),
            Some(body[where_index + "WHERE".len()..].trim()),
        )
    } else {
        (body.trim(), None)
    };

    let match_clause = parse_match_clause(match_part, optional)?;
    let where_clause = where_part.map(parse_where_clause).transpose()?;

    Ok((Some(match_clause), where_clause))
}

/// Parse CREATE body — one or more node patterns.
///
/// Expected input shape: `(n:Label {key: value})` (without the CREATE keyword).
fn parse_create_clause(input: &str) -> Result<CreateClause, ParseError> {
    let (node, rest) = parse_node_pattern(input)?;
    Ok(CreateClause {
        nodes: vec![node],
        relationship: parse_create_relationship(rest)?,
    })
}

// Retain and validate an optional directed edge and target node after CREATE.
fn parse_create_relationship(
    input: &str,
) -> Result<Option<(RelationshipPattern, NodePattern)>, ParseError> {
    parse_directed_relationship(input, "CREATE")
}

/// Parse MERGE body — a single node pattern with optional relationship.
///
/// Expected input shape: `(n:Label {key: value})` (without the MERGE keyword).
fn parse_merge_clause(input: &str) -> Result<MergeClause, ParseError> {
    let (pattern, rest) = parse_node_pattern(input)?;
    Ok(MergeClause {
        pattern,
        relationship: parse_directed_relationship(rest, "MERGE")?,
    })
}

// Retain and validate the optional directed edge and target node after MERGE.
fn parse_directed_relationship(
    input: &str,
    clause: &str,
) -> Result<Option<(RelationshipPattern, NodePattern)>, ParseError> {
    let rest = input.trim();
    if rest.is_empty() {
        return Ok(None);
    }
    let relationship_start = rest.strip_prefix("-[").ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: format!("{clause} relationship must follow the source node as -[...]->(...)"),
        suggestion: None,
    })?;
    let relationship_end = relationship_start.find(']').ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: format!("{clause} relationship must include a closing ']'"),
        suggestion: None,
    })?;
    let relationship = parse_relationship_pattern(&relationship_start[..relationship_end]);
    let target_input = relationship_start[relationship_end + 1..]
        .trim()
        .strip_prefix("->")
        .ok_or_else(|| ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: format!("{clause} relationship must be directed with '->'"),
            suggestion: None,
        })?;
    let (target, trailing) = parse_node_pattern(target_input)?;
    if !trailing.trim().is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: format!("unexpected token after {clause} relationship target"),
            suggestion: None,
        });
    }
    Ok(Some((relationship, target)))
}

/// Parse SET body — comma-separated property assignments.
///
/// Expected input shape: `n.score = 20, n.active = true`
fn parse_set_clause(input: &str) -> Result<SetClause, ParseError> {
    let assignments = input
        .split(',')
        .map(|pair| {
            let (left, right) = pair.split_once('=').ok_or_else(|| ParseError {
                code: ParseErrorCode::InvalidSyntax,
                message: "SET assignment must be shaped as variable.property = value".to_owned(),
                suggestion: None,
            })?;
            let target = parse_property_ref(left.trim())?;
            let value = parse_literal_value(right.trim())?;
            Ok(SetAssignment { target, value })
        })
        .collect::<Result<Vec<SetAssignment>, ParseError>>()?;

    if assignments.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "SET clause must contain at least one assignment".to_owned(),
            suggestion: None,
        });
    }

    Ok(SetClause { assignments })
}

/// Parse DELETE body — comma-separated variable names.
///
/// Expected input shape: `n` or `n, m`
fn parse_delete_clause(input: &str) -> Result<DeleteClause, ParseError> {
    let variables: Vec<String> = input
        .split(',')
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect();

    if variables.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "DELETE clause must specify at least one variable".to_owned(),
            suggestion: None,
        });
    }

    Ok(DeleteClause { variables })
}

/// Parse REMOVE body — comma-separated property references.
///
/// Expected input shape: `n.score` or `n.score, n.active`
fn parse_remove_clause(input: &str) -> Result<RemoveClause, ParseError> {
    let targets = input
        .split(',')
        .map(|prop| parse_property_ref(prop.trim()))
        .collect::<Result<Vec<PropertyRef>, ParseError>>()?;

    if targets.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "REMOVE clause must specify at least one property".to_owned(),
            suggestion: None,
        });
    }

    Ok(RemoveClause { targets })
}

fn parse_match_where_return_query(query_text: &str) -> Result<ParsedQuery, ParseError> {
    let upper = query_text.to_ascii_uppercase();
    let optional_prefix = "OPTIONAL MATCH ";
    let match_prefix = "MATCH ";

    let (optional, body) = if upper.starts_with(optional_prefix) {
        (true, &query_text[optional_prefix.len()..])
    } else if upper.starts_with(match_prefix) {
        (false, &query_text[match_prefix.len()..])
    } else {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "structured parser expects query to start with MATCH or OPTIONAL MATCH"
                .to_owned(),
            suggestion: None,
        });
    };

    let body_upper = body.to_ascii_uppercase();
    let where_pos = find_clause_keyword(&body_upper, "WHERE");
    let return_pos = find_clause_keyword(&body_upper, "RETURN").ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "structured parser requires RETURN clause".to_owned(),
        suggestion: None,
    })?;

    let (match_part, where_part, return_part) = if let Some(where_index) = where_pos {
        if where_index > return_pos {
            return Err(ParseError {
                code: ParseErrorCode::InvalidSyntax,
                message: "WHERE must appear before RETURN in MVP subset".to_owned(),
                suggestion: None,
            });
        }

        (
            body[..where_index].trim(),
            Some(body[where_index + "WHERE".len()..return_pos].trim()),
            body[return_pos + "RETURN".len()..].trim(),
        )
    } else {
        (
            body[..return_pos].trim(),
            None,
            body[return_pos + "RETURN".len()..].trim(),
        )
    };

    let match_clause = parse_match_clause(match_part, optional)?;
    let where_clause = where_part.map(parse_where_clause).transpose()?;
    let return_clause = parse_return_clause(return_part)?;

    Ok(ParsedQuery {
        match_clause: Some(match_clause),
        where_clause,
        return_clause: Some(return_clause),
        create_clause: None,
        merge_clause: None,
        set_clause: None,
        delete_clause: None,
        remove_clause: None,
    })
}

fn parse_match_clause(input: &str, optional: bool) -> Result<MatchClause, ParseError> {
    let (start, rest) = parse_node_pattern(input)?;
    let rest = rest.trim();

    if rest.is_empty() {
        return Ok(MatchClause {
            optional,
            start,
            relationship: None,
        });
    }

    let rest = rest.strip_prefix("-").ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "relationship pattern must start with '-'".to_owned(),
        suggestion: None,
    })?;

    let left = rest.find('[').ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "missing '[' in relationship pattern".to_owned(),
        suggestion: None,
    })?;
    if left != 0 {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "invalid relationship pattern syntax".to_owned(),
            suggestion: None,
        });
    }
    let right = rest.find(']').ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "missing ']' in relationship pattern".to_owned(),
        suggestion: None,
    })?;

    let relationship_inner = &rest[1..right];
    let relationship = parse_relationship_pattern(relationship_inner);

    let after_relationship = rest[right + 1..].trim();
    let after_arrow = after_relationship
        .strip_prefix("->")
        .ok_or_else(|| ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "MVP relationship traversal supports only outgoing '->' direction".to_owned(),
            suggestion: None,
        })?;

    let (end, trailing) = parse_node_pattern(after_arrow.trim())?;
    if !trailing.trim().is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "unexpected tokens after relationship node pattern".to_owned(),
            suggestion: None,
        });
    }

    Ok(MatchClause {
        optional,
        start,
        relationship: Some((relationship, end)),
    })
}

fn parse_node_pattern(input: &str) -> Result<(NodePattern, &str), ParseError> {
    let trimmed = input.trim();
    let left = trimmed.find('(').ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "node pattern must include '('".to_owned(),
        suggestion: None,
    })?;
    if left != 0 {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "unexpected token before node pattern".to_owned(),
            suggestion: None,
        });
    }

    // Find the matching ')' accounting for nested '{...}' blocks.
    let right = find_matching_close_paren(trimmed, left).ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "node pattern must include ')'".to_owned(),
        suggestion: None,
    })?;

    let inner = trimmed[1..right].trim();

    // Split into variable:Label and optional {properties}.
    let (var_label_part, properties) = if let Some(brace_start) = inner.find('{') {
        let props_str = inner[brace_start..].trim();
        let var_label = inner[..brace_start].trim();
        (var_label, parse_inline_properties(props_str)?)
    } else {
        (inner, Vec::new())
    };

    let (variable, label) = if let Some((variable, label)) = var_label_part.split_once(':') {
        (variable.trim(), Some(label.trim().to_owned()))
    } else {
        (var_label_part, None)
    };

    if variable.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "node pattern variable is required in MVP subset".to_owned(),
            suggestion: None,
        });
    }

    Ok((
        NodePattern {
            variable: variable.to_owned(),
            label,
            properties,
        },
        &trimmed[right + 1..],
    ))
}

/// Find the index of the closing ')' that matches the '(' at `open_pos`,
/// skipping over nested `{...}` blocks so inline properties do not confuse the
/// boundary detection.
fn find_matching_close_paren(input: &str, open_pos: usize) -> Option<usize> {
    let mut depth: usize = 0;
    let mut in_braces = false;
    for (i, ch) in input[open_pos..].char_indices() {
        match ch {
            '(' => depth += 1,
            '{' => in_braces = true,
            '}' => in_braces = false,
            ')' if !in_braces => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_pos + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse `{key: value, key2: value2}` into a list of property assignments.
///
/// Expected input shape: `{name: 'alpha', score: 10}` (with braces).
/// Returns an empty vec if the input is empty or contains only whitespace.
fn parse_inline_properties(input: &str) -> Result<Vec<(String, LiteralValue)>, ParseError> {
    let trimmed = input.trim();
    let body = trimmed
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .ok_or_else(|| ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "inline properties must be enclosed in { }".to_owned(),
            suggestion: None,
        })?;

    let body = body.trim();
    if body.is_empty() {
        return Ok(Vec::new());
    }

    body.split(',')
        .map(|pair| {
            let (key, value) = pair.split_once(':').ok_or_else(|| ParseError {
                code: ParseErrorCode::InvalidSyntax,
                message: "inline property must be shaped as key: value".to_owned(),
                suggestion: None,
            })?;
            let key = key.trim().to_owned();
            let value = parse_literal_value(value.trim())?;
            Ok((key, value))
        })
        .collect()
}

fn parse_relationship_pattern(input: &str) -> RelationshipPattern {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return RelationshipPattern {
            variable: None,
            rel_type: None,
        };
    }

    if let Some(rest) = trimmed.strip_prefix(':') {
        return RelationshipPattern {
            variable: None,
            rel_type: Some(rest.trim().to_owned()),
        };
    }

    if let Some((variable, rel_type)) = trimmed.split_once(':') {
        return RelationshipPattern {
            variable: Some(variable.trim().to_owned()),
            rel_type: Some(rel_type.trim().to_owned()),
        };
    }

    RelationshipPattern {
        // Variable.
        variable: Some(trimmed.to_owned()),
        // Rel type.
        rel_type: None,
    }
}

fn parse_where_clause(input: &str) -> Result<WhereClause, ParseError> {
    let operators = [
        (">=", ComparisonOperator::Gte),
        ("<=", ComparisonOperator::Lte),
        ("<>", ComparisonOperator::NotEq),
        ("=", ComparisonOperator::Eq),
        (">", ComparisonOperator::Gt),
        ("<", ComparisonOperator::Lt),
    ];

    for (token, operator) in operators {
        if let Some((left, right)) = split_once_operator(input, token) {
            let left = parse_property_ref(left.trim())?;
            let right = parse_literal_value(right.trim())?;
            return Ok(WhereClause {
                left,
                operator,
                right,
            });
        }
    }

    Err(ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "WHERE clause must use a supported comparison operator".to_owned(),
        suggestion: None,
    })
}

fn split_once_operator<'a>(input: &'a str, operator: &str) -> Option<(&'a str, &'a str)> {
    let index = input.find(operator)?;
    let left = &input[..index];
    let right = &input[index + operator.len()..];
    Some((left, right))
}

fn parse_property_ref(input: &str) -> Result<PropertyRef, ParseError> {
    let (variable, property) = input.split_once('.').ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "property reference must be shaped as variable.property".to_owned(),
        suggestion: None,
    })?;

    if variable.trim().is_empty() || property.trim().is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "property reference contains an empty variable or property".to_owned(),
            suggestion: None,
        });
    }

    Ok(PropertyRef {
        variable: variable.trim().to_owned(),
        property: property.trim().to_owned(),
    })
}

fn parse_literal_value(input: &str) -> Result<LiteralValue, ParseError> {
    let trimmed = input.trim();

    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        return Ok(LiteralValue::String(
            trimmed[1..trimmed.len() - 1].to_owned(),
        ));
    }

    if trimmed.eq_ignore_ascii_case("true") {
        return Ok(LiteralValue::Boolean(true));
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Ok(LiteralValue::Boolean(false));
    }

    if let Ok(value) = trimmed.parse::<i64>() {
        return Ok(LiteralValue::Integer(value));
    }

    Err(ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "literal value is unsupported in MVP parser".to_owned(),
        suggestion: None,
    })
}

fn parse_return_clause(input: &str) -> Result<ReturnClause, ParseError> {
    let upper = input.to_ascii_uppercase();
    let mut cursor = input.trim();
    let distinct = upper.starts_with("DISTINCT ");
    if distinct {
        cursor = cursor["DISTINCT".len()..].trim_start();
    }

    let (items_part, tail_part) = split_projection_and_tail(cursor);
    let items = items_part
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(parse_projection_item)
        .collect::<Result<Vec<ProjectionItem>, ParseError>>()?;

    if items.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "RETURN clause must contain at least one projection item".to_owned(),
            suggestion: None,
        });
    }

    let (order_by, skip, limit) = parse_return_tail(tail_part)?;

    Ok(ReturnClause {
        distinct,
        items,
        order_by,
        skip,
        limit,
    })
}

fn split_projection_and_tail(input: &str) -> (&str, &str) {
    let upper = input.to_ascii_uppercase();
    let order = find_clause_keyword(&upper, "ORDER BY");
    let skip = find_clause_keyword(&upper, "SKIP");
    let limit = find_clause_keyword(&upper, "LIMIT");

    let split_index = [order, skip, limit]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(input.len());

    (&input[..split_index], &input[split_index..])
}

fn parse_projection_item(input: &str) -> Result<ProjectionItem, ParseError> {
    let trimmed = input.trim();
    let upper = trimmed.to_ascii_uppercase();

    if upper.starts_with("COUNT(") && trimmed.ends_with(')') {
        let inner = trimmed["COUNT(".len()..trimmed.len() - 1].trim();
        return Ok(ProjectionItem::Count(inner.to_owned()));
    }

    if trimmed.contains('.') {
        return Ok(ProjectionItem::Property(parse_property_ref(trimmed)?));
    }

    if trimmed.is_empty() {
        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "empty projection item in RETURN clause".to_owned(),
            suggestion: None,
        });
    }

    Ok(ProjectionItem::Variable(trimmed.to_owned()))
}

type ReturnTail = (Option<OrderBy>, Option<usize>, Option<usize>);

fn parse_return_tail(input: &str) -> Result<ReturnTail, ParseError> {
    let mut order_by = None;
    let mut skip = None;
    let mut limit = None;
    let mut cursor = input.trim();

    while !cursor.is_empty() {
        let upper = cursor.to_ascii_uppercase();

        if upper.starts_with("ORDER BY ") {
            let rest = &cursor["ORDER BY".len()..].trim_start();
            let (value, tail) = split_until_next_tail(rest);
            order_by = Some(parse_order_by(value.trim())?);
            cursor = tail.trim_start();
            continue;
        }

        if upper.starts_with("SKIP ") {
            let rest = &cursor["SKIP".len()..].trim_start();
            let (value, tail) = split_first_token(rest);
            skip = Some(value.parse::<usize>().map_err(|_| ParseError {
                code: ParseErrorCode::InvalidSyntax,
                message: "SKIP expects a non-negative integer".to_owned(),
                suggestion: None,
            })?);
            cursor = tail.trim_start();
            continue;
        }

        if upper.starts_with("LIMIT ") {
            let rest = &cursor["LIMIT".len()..].trim_start();
            let (value, tail) = split_first_token(rest);
            limit = Some(value.parse::<usize>().map_err(|_| ParseError {
                code: ParseErrorCode::InvalidSyntax,
                message: "LIMIT expects a non-negative integer".to_owned(),
                suggestion: None,
            })?);
            cursor = tail.trim_start();
            continue;
        }

        return Err(ParseError {
            code: ParseErrorCode::InvalidSyntax,
            message: "unsupported token in RETURN tail".to_owned(),
            suggestion: None,
        });
    }

    Ok((order_by, skip, limit))
}

fn parse_order_by(input: &str) -> Result<OrderBy, ParseError> {
    let mut parts = input.split_whitespace();
    let field = parts.next().ok_or_else(|| ParseError {
        code: ParseErrorCode::InvalidSyntax,
        message: "ORDER BY requires a field".to_owned(),
        suggestion: None,
    })?;
    let direction = match parts.next().map(|part| part.to_ascii_uppercase()) {
        Some(value) if value == "DESC" => OrderDirection::Desc,
        Some(value) if value == "ASC" => OrderDirection::Asc,
        Some(_) => {
            return Err(ParseError {
                code: ParseErrorCode::InvalidSyntax,
                message: "ORDER BY direction must be ASC or DESC".to_owned(),
                suggestion: None,
            });
        }
        None => OrderDirection::Asc,
    };

    Ok(OrderBy {
        field: parse_property_ref(field)?,
        direction,
    })
}

fn split_until_next_tail(input: &str) -> (&str, &str) {
    let upper = input.to_ascii_uppercase();
    let skip = find_clause_keyword(&upper, "SKIP");
    let limit = find_clause_keyword(&upper, "LIMIT");
    let split_index = [skip, limit]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(input.len());
    (&input[..split_index], &input[split_index..])
}

fn split_first_token(input: &str) -> (&str, &str) {
    if let Some(index) = input.find(' ') {
        (&input[..index], &input[index..])
    } else {
        (input, "")
    }
}

fn find_clause_keyword(haystack_upper: &str, keyword_upper: &str) -> Option<usize> {
    if let Some(suffix) = haystack_upper.strip_prefix(keyword_upper)
        && (suffix.is_empty() || suffix.starts_with(' '))
    {
        return Some(0);
    }

    let needle = format!(" {}", keyword_upper);
    haystack_upper.find(&needle).map(|index| index + 1)
}

fn extract_basic_aggregations(
    _query: &Option<ParsedQuery>,
    query_text: &str,
) -> Vec<AggregationFunction> {
    let upper = query_text.to_ascii_uppercase();
    let patterns = [
        ("COUNT(", AggregationFunction::Count),
        ("SUM(", AggregationFunction::Sum),
        ("AVG(", AggregationFunction::Avg),
        ("MIN(", AggregationFunction::Min),
        ("MAX(", AggregationFunction::Max),
    ];

    let mut found: Vec<(usize, AggregationFunction)> = patterns
        .iter()
        .filter_map(|(needle, function)| upper.find(needle).map(|index| (index, function.clone())))
        .collect();
    found.sort_by_key(|(index, _)| *index);
    found.into_iter().map(|(_, function)| function).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_covers_empty_unsupported_and_no_supported_clause_errors() {
        let empty = parse_query(" ").expect_err("empty query should be rejected");
        assert_eq!(empty.code, ParseErrorCode::EmptyQuery);

        let unsupported = parse_query("MATCH (n) UNWIND [1,2] AS x RETURN x")
            .expect_err("unsupported features should be rejected");
        assert_eq!(unsupported.code, ParseErrorCode::UnsupportedFeature);
        assert!(unsupported.message.contains("UNWIND"));

        let no_supported = parse_query("PROFILE n")
            .expect_err("queries with no supported clause should be rejected");
        assert_eq!(no_supported.code, ParseErrorCode::InvalidSyntax);
        assert!(no_supported.message.contains("supported MVP clause"));
    }

    #[test]
    fn parse_query_builds_structured_ast_for_match_where_return() {
        let ast = parse_query(
 "MATCH (n:Campaign)-[r:AMPLIFIES]->(m:Narrative) WHERE n.score >= 10 RETURN DISTINCT n, n.score, COUNT(r) ORDER BY n.score DESC SKIP 1 LIMIT 3",
 )
 .expect("structured MVP query should parse");

        assert_eq!(ast.kind, QueryKind::Read);
        assert!(ast.clauses.contains(&ClauseKind::Match));
        assert!(ast.clauses.contains(&ClauseKind::Where));
        assert!(ast.clauses.contains(&ClauseKind::Return));
        assert_eq!(ast.aggregations, vec![AggregationFunction::Count]);

        let parsed = ast.query.expect("structured query should be attached");
        assert!(parsed.match_clause.is_some());
        assert!(parsed.where_clause.is_some());
        let return_clause = parsed.return_clause.expect("return clause should exist");
        assert!(return_clause.distinct);
        assert_eq!(return_clause.items.len(), 3);
        assert_eq!(return_clause.skip, Some(1));
        assert_eq!(return_clause.limit, Some(3));
        assert!(matches!(
            return_clause.order_by,
            Some(OrderBy {
                direction: OrderDirection::Desc,
                ..
            })
        ));
    }

    #[test]
    fn classify_and_extract_helpers_cover_read_mutation_and_mixed_paths() {
        assert_eq!(
            classify_query_kind(&[ClauseKind::Match, ClauseKind::Return]),
            QueryKind::Read
        );
        assert_eq!(
            classify_query_kind(&[ClauseKind::Create]),
            QueryKind::Mutation
        );
        assert_eq!(
            classify_query_kind(&[ClauseKind::Match, ClauseKind::Create, ClauseKind::Return]),
            QueryKind::Mixed
        );

        let clauses = extract_supported_clauses(
            "OPTIONAL MATCH (n) WHERE n.a = 1 RETURN n ORDER BY n.a SKIP 1 LIMIT 2 CREATE (m)",
        );
        assert_eq!(
            clauses,
            vec![
                ClauseKind::OptionalMatch,
                ClauseKind::Where,
                ClauseKind::Return,
                ClauseKind::OrderBy,
                ClauseKind::Skip,
                ClauseKind::Limit,
                ClauseKind::Create,
            ]
        );
    }

    #[test]
    fn detect_unsupported_feature_identifies_all_mvp_blocked_patterns() {
        assert_eq!(
            detect_unsupported_feature("LOAD CSV FROM 'x'"),
            Some("LOAD CSV")
        );
        assert_eq!(
            detect_unsupported_feature("CALL APOC foo"),
            Some("CALL APOC")
        );
        assert_eq!(
            detect_unsupported_feature("CALL DBMS procedures"),
            Some("CALL DBMS")
        );
        assert_eq!(
            detect_unsupported_feature("MATCH (n) DETACH DELETE n"),
            Some("DETACH DELETE")
        );
        assert_eq!(
            detect_unsupported_feature("UNWIND [1,2] AS n RETURN n"),
            Some("UNWIND")
        );
        assert_eq!(
            detect_unsupported_feature("FOREACH (x IN [1] | RETURN x)"),
            Some("FOREACH")
        );
        assert_eq!(
            detect_unsupported_feature("CALL { MATCH (n) RETURN n }"),
            Some("CALL {")
        );
        assert_eq!(detect_unsupported_feature("MATCH (n) RETURN n"), None);
    }

    #[test]
    fn parse_structured_query_rejects_queries_outside_match_return_shape() {
        let with_clause = parse_structured_query(
            "MATCH (n) WITH n RETURN n",
            &[ClauseKind::Match, ClauseKind::With, ClauseKind::Return],
        )
        .expect_err("WITH clauses are outside the structured parser path");
        assert_eq!(with_clause.code, ParseErrorCode::InvalidSyntax);

        let missing_return = parse_structured_query("MATCH (n)", &[ClauseKind::Match])
            .expect_err("structured parser requires RETURN");
        assert_eq!(missing_return.code, ParseErrorCode::InvalidSyntax);
    }

    #[test]
    fn parse_match_where_return_query_rejects_where_after_return() {
        let error = parse_match_where_return_query("MATCH (n) RETURN n WHERE n.score = 1")
            .expect_err("WHERE must appear before RETURN");
        assert_eq!(error.code, ParseErrorCode::InvalidSyntax);
        assert!(error.message.contains("WHERE must appear before RETURN"));
    }

    #[test]
    fn where_and_property_parsers_cover_supported_operators_and_errors() {
        let operators = vec![
            ("n.a >= 1", ComparisonOperator::Gte),
            ("n.a <= 1", ComparisonOperator::Lte),
            ("n.a <> 1", ComparisonOperator::NotEq),
            ("n.a = 1", ComparisonOperator::Eq),
            ("n.a > 1", ComparisonOperator::Gt),
            ("n.a < 1", ComparisonOperator::Lt),
        ];
        for (input, expected) in operators {
            let clause = parse_where_clause(input).expect("supported operator should parse");
            assert_eq!(clause.operator, expected);
        }

        let invalid_operator =
            parse_where_clause("n.a ~~ 1").expect_err("unsupported operator should be rejected");
        assert_eq!(invalid_operator.code, ParseErrorCode::InvalidSyntax);

        assert!(matches!(
            parse_property_ref("n"),
            Err(ParseError {
                code: ParseErrorCode::InvalidSyntax,
                ..
            })
        ));
        assert!(matches!(
            parse_property_ref("n."),
            Err(ParseError {
                code: ParseErrorCode::InvalidSyntax,
                ..
            })
        ));
    }

    #[test]
    fn return_clause_helpers_cover_projection_tail_and_literal_errors() {
        let return_clause =
            parse_return_clause("n, n.score, COUNT(r) ORDER BY n.score ASC SKIP 2 LIMIT 5")
                .expect("return clause with tail should parse");
        assert_eq!(return_clause.items.len(), 3);
        assert_eq!(return_clause.skip, Some(2));
        assert_eq!(return_clause.limit, Some(5));
        assert!(matches!(
            return_clause.order_by,
            Some(OrderBy {
                direction: OrderDirection::Asc,
                ..
            })
        ));

        let empty_projection =
            parse_return_clause(" ").expect_err("empty return projection should be rejected");
        assert_eq!(empty_projection.code, ParseErrorCode::InvalidSyntax);

        assert_eq!(
            parse_projection_item("n.score").expect("property projection should parse"),
            ProjectionItem::Property(PropertyRef {
                variable: "n".to_owned(),
                property: "score".to_owned(),
            })
        );
        assert_eq!(
            parse_projection_item("COUNT(n)").expect("count projection should parse"),
            ProjectionItem::Count("n".to_owned())
        );
        assert_eq!(
            parse_projection_item("n").expect("variable projection should parse"),
            ProjectionItem::Variable("n".to_owned())
        );

        let literal_error =
            parse_literal_value("3.14").expect_err("unsupported literal shape should be rejected");
        assert_eq!(literal_error.code, ParseErrorCode::InvalidSyntax);
    }

    #[test]
    fn parse_relationship_pattern_covers_all_supported_shapes() {
        assert_eq!(
            parse_relationship_pattern(""),
            RelationshipPattern {
                // Variable.
                variable: None,
                // Rel type.
                rel_type: None,
            }
        );
        assert_eq!(
            parse_relationship_pattern(":AMPLIFIES"),
            RelationshipPattern {
                // Variable.
                variable: None,
                // Rel type.
                rel_type: Some("AMPLIFIES".to_owned()),
            }
        );
        assert_eq!(
            parse_relationship_pattern("r:AMPLIFIES"),
            RelationshipPattern {
                // Variable.
                variable: Some("r".to_owned()),
                // Rel type.
                rel_type: Some("AMPLIFIES".to_owned()),
            }
        );
        assert_eq!(
            parse_relationship_pattern("r"),
            RelationshipPattern {
                // Variable.
                variable: Some("r".to_owned()),
                // Rel type.
                rel_type: None,
            }
        );
    }

    #[test]
    fn parse_literal_value_accepts_string_integer_and_boolean_variants() {
        assert_eq!(
            parse_literal_value("'alpha'").expect("string literal should parse"),
            LiteralValue::String("alpha".to_owned())
        );
        assert_eq!(
            parse_literal_value("42").expect("integer literal should parse"),
            LiteralValue::Integer(42)
        );
        assert_eq!(
            parse_literal_value("TRUE").expect("boolean literal should parse"),
            LiteralValue::Boolean(true)
        );
        assert_eq!(
            parse_literal_value("false").expect("boolean literal should parse"),
            LiteralValue::Boolean(false)
        );
    }

    #[test]
    fn parse_order_by_rejects_invalid_direction_and_missing_field() {
        let invalid_direction =
            parse_order_by("n.score SIDEWAYS").expect_err("invalid direction should be rejected");
        assert_eq!(invalid_direction.code, ParseErrorCode::InvalidSyntax);

        let missing_field = parse_order_by(" ").expect_err("missing field should be rejected");
        assert_eq!(missing_field.code, ParseErrorCode::InvalidSyntax);
    }

    #[test]
    fn parse_return_tail_rejects_unsupported_tokens() {
        let error = parse_return_tail("SKIP 2 OOPS")
            .expect_err("unsupported tail token should be rejected");

        assert_eq!(error.code, ParseErrorCode::InvalidSyntax);
        assert_eq!(error.message, "unsupported token in RETURN tail");
    }

    #[test]
    fn helper_split_and_keyword_detection_paths_are_deterministic() {
        assert_eq!(split_first_token("123 tail"), ("123", " tail"));
        assert_eq!(split_first_token("123"), ("123", ""));

        assert_eq!(
            split_until_next_tail("n.score DESC SKIP 1 LIMIT 2"),
            ("n.score DESC ", "SKIP 1 LIMIT 2")
        );
        assert_eq!(split_until_next_tail("n.score DESC"), ("n.score DESC", ""));

        assert_eq!(find_clause_keyword("ORDER BY n.score", "ORDER BY"), Some(0));
        assert_eq!(
            find_clause_keyword("RETURN n ORDER BY n.score", "ORDER BY"),
            Some(9)
        );
        assert_eq!(find_clause_keyword("RETURN n", "LIMIT"), None);
    }

    #[test]
    fn parse_match_clause_rejects_incoming_relationship_direction() {
        let error = parse_match_clause("(a)<-[:REL]-(b)", false)
            .expect_err("incoming direction should be rejected in MVP subset");

        assert_eq!(error.code, ParseErrorCode::InvalidSyntax);
        assert!(
            error
                .message
                .contains("relationship pattern must start with '-'")
        );
    }

    #[test]
    fn parse_node_pattern_rejects_empty_variable_and_unexpected_prefix() {
        let empty_variable =
            parse_node_pattern("(:Indicator)").expect_err("empty node variable should be rejected");
        assert_eq!(empty_variable.code, ParseErrorCode::InvalidSyntax);

        let prefixed = parse_node_pattern("x(n)")
            .expect_err("unexpected token before node pattern should be rejected");
        assert_eq!(prefixed.code, ParseErrorCode::InvalidSyntax);
    }
}
