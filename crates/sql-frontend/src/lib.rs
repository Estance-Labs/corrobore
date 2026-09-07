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
//! SQL frontend over the shared Corrobore query AST (epic #90, item #259).
//!
//! Crate boundary: this crate turns a documented SQL subset into the same
//! structural [`QueryAst`] the Cypher parser produces, and says how a SQL row
//! is projected from the executor's typed record values. It executes nothing,
//! opens no socket and never renders Cypher text: SQL and Cypher meet at the
//! AST, and the planner and executor treat both alike.
//!
//! The relational projection is one table per node label (plus `nodes` for
//! every label and `relationships` for every type), with the columns `id`,
//! `labels` or `type`, `source_id` and `target_id` for relationships, the
//! native `status`, `confidence` and `evidence_refs`, a `properties` JSON
//! document, and every property as its own column. A relationship is joined
//! on its endpoints: `JOIN "USES" AS r ON r.source_id = a.id JOIN "Malware"
//! AS m ON r.target_id = m.id` is the one traversal shape the shared AST
//! carries, and it compiles to the pattern `(a)-[r:USES]->(m)`.

use std::collections::{BTreeMap, HashMap};

use cypher_executor::RecordValue;
use cypher_parser::{
    ComparisonOperator, CreateClause, DeleteClause, LiteralValue, MatchClause, NodePattern,
    OrderBy, OrderDirection, ParsedQuery, ProjectionItem, PropertyRef, QueryAst,
    RelationshipPattern, ReturnClause, SetAssignment, SetClause, WhereClause, WhereExpression,
};

/// What the server announces as `server_version`; the PostgreSQL major keeps
/// drivers' feature detection sane and the product name says who answers.
pub const SERVER_VERSION: &str = concat!("16.0 (Corrobore ", env!("CARGO_PKG_VERSION"), ")");

/// A parameter value bound by the protocol layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SqlValue {
    /// SQL NULL.
    Null,
    /// Boolean.
    Boolean(bool),
    /// Integer.
    Integer(i64),
    /// Decimal as lossless text.
    Float(String),
    /// Text.
    Text(String),
}

/// One compiled statement.
///
/// `Query` is by far the largest variant, and it is also the common one; the
/// enum is built once per statement, so boxing it would cost an allocation on
/// the hot path to save bytes on the rare control statements.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum SqlCommand {
    /// A graph query or mutation compiled into the shared AST.
    Query(CompiledQuery),
    /// A `SELECT` without `FROM`, answered locally.
    Scalar(ScalarResult),
    /// An `information_schema` read, answered from the graph by the adapter.
    Catalog(CatalogQuery),
    /// `BEGIN` / `START TRANSACTION`.
    Begin {
        /// `READ ONLY` was requested.
        read_only: bool,
    },
    /// `COMMIT` / `END`.
    Commit,
    /// `ROLLBACK` / `ABORT`.
    Rollback,
    /// `SET` / `RESET` / `DISCARD`, accepted and ignored.
    Set,
    /// `SHOW <name>`.
    Show(String),
    /// Nothing but whitespace.
    Empty,
}

/// Read or write, decided from the statement kind and used by the adapter to
/// pick the runtime request mode explicitly instead of scanning keywords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryMode {
    /// A read.
    Read,
    /// A mutation.
    Mutation,
}

/// Which `CommandComplete` tag a statement produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandTag {
    /// `SELECT n`.
    Select,
    /// `INSERT 0 n`.
    Insert,
    /// `UPDATE n`.
    Update,
    /// `DELETE n`.
    Delete,
}

/// A graph statement compiled into the shared AST.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledQuery {
    /// The shared structural AST the planner and executor consume.
    pub ast: QueryAst,
    /// Read or mutation.
    pub mode: QueryMode,
    /// SQL result columns, in `SELECT` order, and where each comes from.
    pub columns: Vec<OutputColumn>,
    /// The command tag family.
    pub tag: CommandTag,
    /// Work the adapter applies after execution when it could not be pushed
    /// into the AST (see [`CompiledQuery::columns`]).
    pub post: PostProcess,
}

/// Result work applied by the adapter after execution.
///
/// When every column is a property projection, `DISTINCT`, `OFFSET` and
/// `LIMIT` live in the AST and this is empty. When a column needs a whole
/// record (structural fields, `*`), the AST returns whole variables and the
/// row-level work happens here, on the bounded result the executor returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostProcess {
    /// Drop duplicate rows after projection.
    pub distinct: bool,
    /// Skip this many rows.
    pub offset: usize,
    /// Keep at most this many rows.
    pub limit: Option<usize>,
}

/// One SQL result column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputColumn {
    /// The column name a client sees (alias or derived).
    pub name: String,
    /// Where the cell comes from.
    pub source: ColumnSource,
}

/// Where a SQL cell comes from in a typed record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnSource {
    /// A record field keyed the way the executor names it (`a.name`, `count`).
    Field(String),
    /// A field of a whole node or relationship the record carries.
    Entity {
        /// The record variable.
        variable: String,
        /// Which part of it.
        field: EntityField,
    },
    /// A constant, for scalar selects.
    Constant(Cell),
}

/// Parts of a node or relationship exposed as columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityField {
    /// Record identifier.
    Id,
    /// Node labels as a JSON array.
    Labels,
    /// Relationship type.
    Type,
    /// Relationship source identifier.
    SourceId,
    /// Relationship target identifier.
    TargetId,
    /// Lifecycle status.
    Status,
    /// Confidence.
    Confidence,
    /// Evidence references as a JSON array.
    EvidenceRefs,
    /// Every application property as one JSON object.
    Properties,
    /// One property.
    Property(String),
}

/// A `SELECT` without `FROM`, fully evaluated at compile time.
#[derive(Clone, Debug, PartialEq)]
pub struct ScalarResult {
    /// Result columns.
    pub columns: Vec<OutputColumn>,
    /// The single row.
    pub row: Vec<Cell>,
}

/// Which catalog view is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogKind {
    /// `information_schema.tables`.
    Tables,
    /// `information_schema.columns`.
    Columns,
}

/// An `information_schema` read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogQuery {
    /// The view.
    pub kind: CatalogKind,
    /// `table_name = '...'` filter, when present.
    pub table: Option<String>,
    /// Requested column names, in `SELECT` order; empty means every column.
    pub columns: Vec<String>,
    /// `ORDER BY <column>` when present.
    pub order_by: Option<String>,
}

/// A typed cell of a SQL row.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// SQL NULL.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Decimal as lossless text.
    Float(String),
    /// Text.
    Text(String),
    /// A JSON document (lists, maps, whole records).
    Json(serde_json::Value),
}

impl Eq for Cell {}

/// The PostgreSQL type a column is described with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    /// `bool`.
    Bool,
    /// `int8`.
    Int8,
    /// `float8`.
    Float8,
    /// `text`.
    Text,
    /// `jsonb`.
    Jsonb,
}

impl ColumnType {
    /// PostgreSQL type OID.
    #[must_use]
    pub const fn oid(self) -> i32 {
        match self {
            Self::Bool => 16,
            Self::Int8 => 20,
            Self::Text => 25,
            Self::Float8 => 701,
            Self::Jsonb => 3802,
        }
    }

    /// Type length for a row description, `-1` for variable width.
    #[must_use]
    pub const fn length(self) -> i16 {
        match self {
            Self::Bool => 1,
            Self::Int8 | Self::Float8 => 8,
            Self::Text | Self::Jsonb => -1,
        }
    }

    /// The `information_schema.columns.data_type` name.
    #[must_use]
    pub const fn data_type_name(self) -> &'static str {
        match self {
            Self::Bool => "boolean",
            Self::Int8 => "bigint",
            Self::Float8 => "double precision",
            Self::Text => "text",
            Self::Jsonb => "jsonb",
        }
    }
}

/// A compile failure with the SQLSTATE a client acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlError {
    /// Five-character SQLSTATE.
    pub sqlstate: &'static str,
    /// Human-readable message; never contains configuration or secrets.
    pub message: String,
}

impl std::fmt::Display for SqlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} ({})", self.message, self.sqlstate)
    }
}

impl std::error::Error for SqlError {}

/// SQLSTATE codes this crate emits.
pub mod sqlstate {
    /// `syntax_error`.
    pub const SYNTAX_ERROR: &str = "42601";
    /// `undefined_table`.
    pub const UNDEFINED_TABLE: &str = "42P01";
    /// `undefined_column`.
    pub const UNDEFINED_COLUMN: &str = "42703";
    /// `ambiguous_column`.
    pub const AMBIGUOUS_COLUMN: &str = "42702";
    /// `grouping_error`.
    pub const GROUPING_ERROR: &str = "42803";
    /// `feature_not_supported`.
    pub const FEATURE_NOT_SUPPORTED: &str = "0A000";
    /// `invalid_parameter_value`.
    pub const INVALID_PARAMETER_VALUE: &str = "22023";
    /// `datatype_mismatch`.
    pub const DATATYPE_MISMATCH: &str = "42804";
}

fn error(sqlstate: &'static str, message: impl Into<String>) -> SqlError {
    SqlError {
        sqlstate,
        message: message.into(),
    }
}

fn unsupported(what: &str) -> SqlError {
    error(
        sqlstate::FEATURE_NOT_SUPPORTED,
        format!("{what} is not supported by the Corrobore SQL frontend"),
    )
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A bare or quoted identifier; bare ones keep their case.
    Ident {
        text: String,
        quoted: bool,
    },
    Number(String),
    Str(String),
    /// `$1` or `$name`.
    Param(String),
    /// Punctuation and operators.
    Symbol(&'static str),
}

const SYMBOLS: [&str; 16] = [
    "<>", "!=", "<=", ">=", "(", ")", ",", ".", "*", "=", "<", ">", "[", "]", ";", "::",
];

fn tokenize(sql: &str) -> Result<Vec<Token>, SqlError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        if character == '-' && chars.get(index + 1) == Some(&'-') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if character == '\'' {
            let mut text = String::new();
            index += 1;
            loop {
                match chars.get(index) {
                    None => {
                        return Err(error(sqlstate::SYNTAX_ERROR, "unterminated string literal"));
                    }
                    Some('\'') if chars.get(index + 1) == Some(&'\'') => {
                        text.push('\'');
                        index += 2;
                    }
                    Some('\'') => {
                        index += 1;
                        break;
                    }
                    Some(other) => {
                        text.push(*other);
                        index += 1;
                    }
                }
            }
            tokens.push(Token::Str(text));
            continue;
        }
        if character == '"' {
            let mut text = String::new();
            index += 1;
            loop {
                match chars.get(index) {
                    None => {
                        return Err(error(
                            sqlstate::SYNTAX_ERROR,
                            "unterminated quoted identifier",
                        ));
                    }
                    Some('"') if chars.get(index + 1) == Some(&'"') => {
                        text.push('"');
                        index += 2;
                    }
                    Some('"') => {
                        index += 1;
                        break;
                    }
                    Some(other) => {
                        text.push(*other);
                        index += 1;
                    }
                }
            }
            tokens.push(Token::Ident { text, quoted: true });
            continue;
        }
        if character == '$' {
            let start = index + 1;
            index += 1;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            if index == start {
                return Err(error(
                    sqlstate::SYNTAX_ERROR,
                    "a parameter needs a number or a name after `$`",
                ));
            }
            tokens.push(Token::Param(chars[start..index].iter().collect()));
            continue;
        }
        if character.is_ascii_digit()
            || (character == '-'
                && chars.get(index + 1).is_some_and(char::is_ascii_digit)
                && !matches!(
                    tokens.last(),
                    Some(Token::Number(_) | Token::Ident { .. } | Token::Symbol(")"))
                ))
        {
            let start = index;
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_digit()
                    || chars[index] == '.'
                    || chars[index] == 'e'
                    || chars[index] == 'E'
                    || ((chars[index] == '-' || chars[index] == '+')
                        && matches!(chars[index - 1], 'e' | 'E')))
            {
                index += 1;
            }
            tokens.push(Token::Number(chars[start..index].iter().collect()));
            continue;
        }
        if character.is_alphabetic() || character == '_' {
            let start = index;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            tokens.push(Token::Ident {
                text: chars[start..index].iter().collect(),
                quoted: false,
            });
            continue;
        }
        let rest: String = chars[index..(index + 2).min(chars.len())].iter().collect();
        if let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) {
            tokens.push(Token::Symbol(symbol));
            index += symbol.len();
            continue;
        }
        return Err(error(
            sqlstate::SYNTAX_ERROR,
            format!("unexpected character `{character}` at position {index}"),
        ));
    }
    Ok(tokens)
}

/// Split a simple-query string into statements on `;` outside quotes,
/// dropping empty ones.
#[must_use]
pub fn split_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for character in sql.chars() {
        match quote {
            Some(open) => {
                current.push(character);
                if character == open {
                    quote = None;
                }
            }
            None => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    current.push(character);
                }
                ';' => {
                    if !current.trim().is_empty() {
                        statements.push(current.trim().to_owned());
                    }
                    current.clear();
                }
                _ => current.push(character),
            },
        }
    }
    if !current.trim().is_empty() {
        statements.push(current.trim().to_owned());
    }
    statements
}

/// The number of positional parameters (`$1`..`$n`) a statement declares.
#[must_use]
pub fn parameter_count(sql: &str) -> usize {
    tokenize(sql)
        .map(|tokens| {
            tokens
                .iter()
                .filter_map(|token| match token {
                    Token::Param(name) => name.parse::<usize>().ok(),
                    _ => None,
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser<'a> {
    tokens: Vec<Token>,
    position: usize,
    parameters: &'a [SqlValue],
    /// Named parameters are numbered after the positional ones in first-use
    /// order, so `$who` after `$1` and `$2` is the third value.
    named: Vec<String>,
    positional_max: usize,
    source: &'a str,
}

#[derive(Clone, Debug)]
struct TableRef {
    name: String,
    alias: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Column {
        qualifier: Option<String>,
        name: String,
    },
    Literal(LiteralValue),
    Aggregate {
        function: String,
        argument: Option<Box<Expr>>,
    },
    Function(String),
    Star,
}

#[derive(Clone, Debug)]
struct SelectItem {
    expr: Expr,
    alias: Option<String>,
}

#[derive(Clone, Debug)]
enum Condition {
    Comparison {
        left: Expr,
        operator: ComparisonOperator,
        right: Expr,
    },
    In {
        left: Expr,
        values: Vec<LiteralValue>,
        negated: bool,
    },
    IsNull {
        left: Expr,
        negated: bool,
    },
    And(Vec<Condition>),
    Or(Vec<Condition>),
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, parameters: &'a [SqlValue]) -> Result<Self, SqlError> {
        Ok(Self {
            tokens: tokenize(source)?,
            position: 0,
            parameters,
            named: Vec::new(),
            positional_max: 0,
            source,
        })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.position + offset)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn is_keyword(&self, offset: usize, keyword: &str) -> bool {
        matches!(
            self.peek_at(offset),
            Some(Token::Ident { text, quoted: false }) if text.eq_ignore_ascii_case(keyword)
        )
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        if self.is_keyword(0, keyword) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), SqlError> {
        if self.eat_keyword(keyword) {
            Ok(())
        } else {
            Err(self.syntax(&format!("expected `{keyword}`")))
        }
    }

    fn eat_symbol(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Symbol(found)) if *found == symbol) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect_symbol(&mut self, symbol: &str) -> Result<(), SqlError> {
        if self.eat_symbol(symbol) {
            Ok(())
        } else {
            Err(self.syntax(&format!("expected `{symbol}`")))
        }
    }

    fn syntax(&self, message: &str) -> SqlError {
        let near = match self.peek() {
            Some(Token::Ident { text, .. })
            | Some(Token::Number(text))
            | Some(Token::Str(text)) => {
                format!(" near `{text}`")
            }
            Some(Token::Param(name)) => format!(" near `${name}`"),
            Some(Token::Symbol(symbol)) => format!(" near `{symbol}`"),
            None => " at end of input".to_owned(),
        };
        error(sqlstate::SYNTAX_ERROR, format!("{message}{near}"))
    }

    fn identifier(&mut self) -> Result<String, SqlError> {
        match self.next() {
            Some(Token::Ident { text, .. }) => Ok(text),
            _ => {
                self.position = self.position.saturating_sub(1);
                Err(self.syntax("expected an identifier"))
            }
        }
    }

    fn at_end(&self) -> bool {
        self.peek().is_none() || matches!(self.peek(), Some(Token::Symbol(";")))
    }

    fn expect_end(&self) -> Result<(), SqlError> {
        if self.at_end() {
            Ok(())
        } else {
            Err(self.syntax("unexpected input"))
        }
    }

    // -- statements -----------------------------------------------------------

    fn statement(&mut self) -> Result<SqlCommand, SqlError> {
        if self.at_end() {
            return Ok(SqlCommand::Empty);
        }
        let Some(Token::Ident {
            text,
            quoted: false,
        }) = self.peek().cloned()
        else {
            return Err(self.syntax("expected a statement"));
        };
        match text.to_ascii_uppercase().as_str() {
            "SELECT" => self.select(),
            "INSERT" => self.insert(),
            "UPDATE" => self.update(),
            "DELETE" => self.delete(),
            "BEGIN" | "START" => self.begin(),
            "COMMIT" | "END" => {
                self.position += 1;
                self.transaction_noise();
                self.expect_end()?;
                Ok(SqlCommand::Commit)
            }
            "ROLLBACK" | "ABORT" => {
                self.position += 1;
                self.transaction_noise();
                self.expect_end()?;
                Ok(SqlCommand::Rollback)
            }
            "SET" | "RESET" | "DISCARD" => Ok(SqlCommand::Set),
            "SHOW" => {
                self.position += 1;
                let name = self.identifier()?;
                self.expect_end()?;
                Ok(SqlCommand::Show(name.to_ascii_lowercase()))
            }
            "WITH" => Err(unsupported("common table expressions (WITH)")),
            "CREATE" | "DROP" | "ALTER" | "TRUNCATE" | "GRANT" | "REVOKE" | "COPY" | "VACUUM"
            | "ANALYZE" | "EXPLAIN" | "LISTEN" | "NOTIFY" | "DECLARE" | "FETCH" | "PREPARE"
            | "EXECUTE" | "DEALLOCATE" | "LOCK" | "MERGE" | "CALL" | "DO" | "VALUES" | "TABLE" => {
                Err(unsupported(&format!("`{}`", text.to_ascii_uppercase())))
            }
            _ => Err(self.syntax("expected a statement")),
        }
    }

    fn transaction_noise(&mut self) {
        while self.eat_keyword("WORK") || self.eat_keyword("TRANSACTION") {}
    }

    fn begin(&mut self) -> Result<SqlCommand, SqlError> {
        if self.eat_keyword("START") {
            self.expect_keyword("TRANSACTION")?;
        } else {
            self.position += 1;
            self.transaction_noise();
        }
        let mut read_only = false;
        while !self.at_end() {
            if self.eat_keyword("READ") {
                if self.eat_keyword("ONLY") {
                    read_only = true;
                } else if !self.eat_keyword("WRITE") {
                    return Err(self.syntax("expected `ONLY` or `WRITE`"));
                }
            } else if self.eat_keyword("ISOLATION") {
                self.expect_keyword("LEVEL")?;
                // Any isolation level is accepted: every statement is its own
                // atomic request, and the documentation says so.
                while !self.at_end() && !self.is_keyword(0, "READ") && !self.eat_symbol(",") {
                    self.position += 1;
                }
            } else if self.eat_keyword("NOT") {
                self.expect_keyword("DEFERRABLE")?;
            } else if self.eat_keyword("DEFERRABLE") || self.eat_symbol(",") {
            } else {
                return Err(self.syntax("unexpected transaction option"));
            }
        }
        Ok(SqlCommand::Begin { read_only })
    }

    // -- SELECT ---------------------------------------------------------------

    fn select(&mut self) -> Result<SqlCommand, SqlError> {
        self.expect_keyword("SELECT")?;
        let distinct = if self.eat_keyword("DISTINCT") {
            true
        } else {
            self.eat_keyword("ALL");
            false
        };
        let items = self.select_items()?;
        if !self.eat_keyword("FROM") {
            self.expect_end()?;
            return self.scalar(items);
        }
        // information_schema.<view>
        if self.is_keyword(0, "information_schema")
            && matches!(self.peek_at(1), Some(Token::Symbol(".")))
        {
            self.position += 2;
            let view = self.identifier()?;
            let kind = match view.to_ascii_lowercase().as_str() {
                "tables" => CatalogKind::Tables,
                "columns" => CatalogKind::Columns,
                other => return Err(unsupported(&format!("information_schema.{other}"))),
            };
            return self.catalog(kind, items);
        }
        let mut tables = vec![self.table_ref()?];
        let mut joins: Vec<(Expr, Expr)> = Vec::new();
        loop {
            if self.eat_symbol(",") {
                return Err(unsupported("cross joins (comma-separated FROM items)"));
            }
            let explicit_join = self.eat_keyword("INNER");
            if self.eat_keyword("LEFT")
                || self.eat_keyword("RIGHT")
                || self.eat_keyword("FULL")
                || self.eat_keyword("CROSS")
            {
                return Err(unsupported("outer and cross joins"));
            }
            if self.eat_keyword("JOIN") {
                tables.push(self.table_ref()?);
                self.expect_keyword("ON")?;
                let left = self.column_expr()?;
                self.expect_symbol("=")?;
                let right = self.column_expr()?;
                joins.push((left, right));
                continue;
            }
            if explicit_join {
                return Err(self.syntax("expected `JOIN`"));
            }
            break;
        }
        let condition = if self.eat_keyword("WHERE") {
            Some(self.condition()?)
        } else {
            None
        };
        if self.eat_keyword("GROUP") {
            return Err(unsupported("GROUP BY"));
        }
        if self.eat_keyword("HAVING") {
            return Err(unsupported("HAVING"));
        }
        if self.eat_keyword("UNION") || self.eat_keyword("INTERSECT") || self.eat_keyword("EXCEPT")
        {
            return Err(unsupported("set operations"));
        }
        let mut order_by = Vec::new();
        if self.eat_keyword("ORDER") {
            self.expect_keyword("BY")?;
            loop {
                let expr = self.expr()?;
                let direction = if self.eat_keyword("DESC") {
                    OrderDirection::Desc
                } else {
                    self.eat_keyword("ASC");
                    OrderDirection::Asc
                };
                if self.eat_keyword("NULLS")
                    && !(self.eat_keyword("FIRST") || self.eat_keyword("LAST"))
                {
                    return Err(self.syntax("expected `FIRST` or `LAST`"));
                }
                order_by.push((expr, direction));
                if !self.eat_symbol(",") {
                    break;
                }
            }
        }
        let mut limit = None;
        let mut offset = None;
        loop {
            if self.eat_keyword("LIMIT") {
                limit = if self.eat_keyword("ALL") {
                    None
                } else {
                    self.count()?
                };
            } else if self.eat_keyword("OFFSET") {
                offset = self.count()?;
                if !self.eat_keyword("ROWS") {
                    self.eat_keyword("ROW");
                }
            } else if self.eat_keyword("FOR") {
                return Err(unsupported("row locking clauses"));
            } else {
                break;
            }
        }
        self.expect_end()?;
        let scope = Scope::new(&tables, &joins)?;
        compile_select(
            self.source,
            &scope,
            distinct,
            items,
            condition,
            order_by,
            limit,
            offset,
        )
    }

    fn select_items(&mut self) -> Result<Vec<SelectItem>, SqlError> {
        let mut items = Vec::new();
        loop {
            let expr = self.expr()?;
            let alias = if self.eat_keyword("AS") {
                Some(self.identifier()?)
            } else if let Some(Token::Ident { text, quoted }) = self.peek().cloned() {
                if quoted || !is_reserved(&text) {
                    self.position += 1;
                    Some(text)
                } else {
                    None
                }
            } else {
                None
            };
            items.push(SelectItem { expr, alias });
            if !self.eat_symbol(",") {
                break;
            }
        }
        Ok(items)
    }

    fn table_ref(&mut self) -> Result<TableRef, SqlError> {
        if self.is_keyword(0, "SELECT") || matches!(self.peek(), Some(Token::Symbol("("))) {
            return Err(unsupported("subqueries in FROM"));
        }
        let mut name = self.identifier()?;
        if self.eat_symbol(".") {
            // schema.table: the schema is accepted and ignored (there is one).
            name = self.identifier()?;
        }
        let alias = if self.eat_keyword("AS") {
            self.identifier()?
        } else if let Some(Token::Ident { text, quoted }) = self.peek().cloned() {
            if quoted || !is_reserved(&text) {
                self.position += 1;
                text
            } else {
                name.clone()
            }
        } else {
            name.clone()
        };
        Ok(TableRef { name, alias })
    }

    /// A LIMIT or OFFSET count. A NULL here is an unbound placeholder from a
    /// Parse-time syntax check and means "no bound" until Bind supplies one.
    fn count(&mut self) -> Result<Option<usize>, SqlError> {
        match self.expr()? {
            Expr::Literal(LiteralValue::Integer(value)) if value >= 0 => usize::try_from(value)
                .map(Some)
                .map_err(|_| self.syntax("count out of range")),
            Expr::Literal(LiteralValue::Null) => Ok(None),
            _ => Err(error(
                sqlstate::DATATYPE_MISMATCH,
                "LIMIT and OFFSET take a non-negative integer",
            )),
        }
    }

    // -- expressions -----------------------------------------------------------

    fn column_expr(&mut self) -> Result<Expr, SqlError> {
        match self.expr()? {
            column @ Expr::Column { .. } => Ok(column),
            _ => Err(self.syntax("expected a column reference")),
        }
    }

    fn expr(&mut self) -> Result<Expr, SqlError> {
        let expr = match self.next() {
            Some(Token::Symbol("*")) => Expr::Star,
            Some(Token::Symbol("(")) => {
                let inner = self.expr()?;
                self.expect_symbol(")")?;
                inner
            }
            Some(Token::Number(text)) => Expr::Literal(number_literal(&text)?),
            Some(Token::Str(text)) => Expr::Literal(LiteralValue::String(text)),
            Some(Token::Param(name)) => Expr::Literal(self.parameter(&name)?),
            Some(Token::Ident {
                text,
                quoted: false,
            }) if text.eq_ignore_ascii_case("TRUE") => Expr::Literal(LiteralValue::Boolean(true)),
            Some(Token::Ident {
                text,
                quoted: false,
            }) if text.eq_ignore_ascii_case("FALSE") => Expr::Literal(LiteralValue::Boolean(false)),
            Some(Token::Ident {
                text,
                quoted: false,
            }) if text.eq_ignore_ascii_case("NULL") => Expr::Literal(LiteralValue::Null),
            Some(Token::Ident {
                text,
                quoted: false,
            }) if text.eq_ignore_ascii_case("ARRAY") => {
                self.expect_symbol("[")?;
                let mut values = Vec::new();
                if !self.eat_symbol("]") {
                    loop {
                        match self.expr()? {
                            Expr::Literal(value) => values.push(value),
                            _ => return Err(self.syntax("ARRAY elements must be literals")),
                        }
                        if self.eat_symbol("]") {
                            break;
                        }
                        self.expect_symbol(",")?;
                    }
                }
                Expr::Literal(LiteralValue::List(values))
            }
            Some(Token::Ident { text, quoted }) => {
                let upper = text.to_ascii_uppercase();
                if !quoted && matches!(self.peek(), Some(Token::Symbol("("))) {
                    self.position += 1;
                    match upper.as_str() {
                        "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" => {
                            let argument = if self.eat_symbol("*") {
                                None
                            } else {
                                Some(Box::new(self.expr()?))
                            };
                            self.expect_symbol(")")?;
                            Expr::Aggregate {
                                function: upper.to_ascii_lowercase(),
                                argument,
                            }
                        }
                        _ => {
                            if !self.eat_symbol(")") {
                                return Err(unsupported(&format!(
                                    "function `{text}` with arguments"
                                )));
                            }
                            Expr::Function(text.to_ascii_lowercase())
                        }
                    }
                } else if !quoted
                    && matches!(
                        upper.as_str(),
                        "CURRENT_USER"
                            | "SESSION_USER"
                            | "CURRENT_SCHEMA"
                            | "CURRENT_CATALOG"
                            | "CURRENT_ROLE"
                    )
                {
                    Expr::Function(text.to_ascii_lowercase())
                } else if self.eat_symbol(".") {
                    if self.eat_symbol("*") {
                        return Err(unsupported("`alias.*`; use `*` or name the columns"));
                    }
                    let name = self.identifier()?;
                    Expr::Column {
                        qualifier: Some(text),
                        name,
                    }
                } else {
                    Expr::Column {
                        qualifier: None,
                        name: text,
                    }
                }
            }
            Some(other) => {
                self.position -= 1;
                return Err(self.syntax(&format!("unexpected token {other:?}")));
            }
            None => return Err(self.syntax("unexpected end of statement")),
        };
        // A `::type` cast is accepted and ignored: cells are typed by value.
        if self.eat_symbol("::") {
            self.identifier()?;
            while self.eat_symbol("[") {
                self.expect_symbol("]")?;
            }
        }
        Ok(expr)
    }

    fn parameter(&mut self, name: &str) -> Result<LiteralValue, SqlError> {
        let index = match name.parse::<usize>() {
            Ok(0) => {
                return Err(error(
                    sqlstate::SYNTAX_ERROR,
                    "parameters are numbered from $1",
                ));
            }
            Ok(position) => {
                self.positional_max = self.positional_max.max(position);
                position - 1
            }
            Err(_) => {
                let named_index = match self.named.iter().position(|known| known == name) {
                    Some(position) => position,
                    None => {
                        self.named.push(name.to_owned());
                        self.named.len() - 1
                    }
                };
                self.positional_max_after_named() + named_index
            }
        };
        let value = self.parameters.get(index).ok_or_else(|| {
            error(
                sqlstate::INVALID_PARAMETER_VALUE,
                format!("parameter ${name} has no bound value"),
            )
        })?;
        Ok(match value {
            SqlValue::Null => LiteralValue::Null,
            SqlValue::Boolean(value) => LiteralValue::Boolean(*value),
            SqlValue::Integer(value) => LiteralValue::Integer(*value),
            SqlValue::Float(text) => LiteralValue::Float(text.clone()),
            SqlValue::Text(text) => LiteralValue::String(text.clone()),
        })
    }

    /// Named parameters follow every positional one the statement uses.
    fn positional_max_after_named(&self) -> usize {
        let positional_in_statement = self
            .tokens
            .iter()
            .filter_map(|token| match token {
                Token::Param(name) => name.parse::<usize>().ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        positional_in_statement.max(self.positional_max)
    }

    fn condition(&mut self) -> Result<Condition, SqlError> {
        let mut terms = vec![self.and_condition()?];
        while self.eat_keyword("OR") {
            terms.push(self.and_condition()?);
        }
        Ok(if terms.len() == 1 {
            terms.remove(0)
        } else {
            Condition::Or(terms)
        })
    }

    fn and_condition(&mut self) -> Result<Condition, SqlError> {
        let mut terms = vec![self.predicate()?];
        while self.eat_keyword("AND") {
            terms.push(self.predicate()?);
        }
        Ok(if terms.len() == 1 {
            terms.remove(0)
        } else {
            Condition::And(terms)
        })
    }

    fn predicate(&mut self) -> Result<Condition, SqlError> {
        if self.eat_keyword("NOT") {
            return Err(unsupported("NOT (use `<>`, `NOT IN` or `IS NOT NULL`)"));
        }
        if matches!(self.peek(), Some(Token::Symbol("("))) {
            // Parenthesised condition or a parenthesised expression: try the
            // condition first, which is what the boolean grammar means here.
            let saved = self.position;
            self.position += 1;
            if let Ok(inner) = self.condition()
                && self.eat_symbol(")")
                && !matches!(
                    self.peek(),
                    Some(Token::Symbol("=" | "<>" | "!=" | "<" | "<=" | ">" | ">="))
                )
            {
                return Ok(inner);
            }
            self.position = saved;
        }
        let left = self.expr()?;
        if self.eat_keyword("IS") {
            let negated = self.eat_keyword("NOT");
            self.expect_keyword("NULL")?;
            return Ok(Condition::IsNull { left, negated });
        }
        let negated = self.eat_keyword("NOT");
        if self.eat_keyword("IN") {
            self.expect_symbol("(")?;
            let mut values = Vec::new();
            loop {
                match self.expr()? {
                    Expr::Literal(value) => values.push(value),
                    _ => return Err(self.syntax("IN takes literal values")),
                }
                if self.eat_symbol(")") {
                    break;
                }
                self.expect_symbol(",")?;
            }
            return Ok(Condition::In {
                left,
                values,
                negated,
            });
        }
        if negated {
            return Err(self.syntax("expected `IN` after `NOT`"));
        }
        if self.eat_keyword("LIKE") || self.eat_keyword("ILIKE") || self.eat_keyword("BETWEEN") {
            return Err(unsupported("LIKE, ILIKE and BETWEEN"));
        }
        let operator = match self.next() {
            Some(Token::Symbol("=")) => ComparisonOperator::Eq,
            Some(Token::Symbol("<>" | "!=")) => ComparisonOperator::NotEq,
            Some(Token::Symbol("<")) => ComparisonOperator::Lt,
            Some(Token::Symbol("<=")) => ComparisonOperator::Lte,
            Some(Token::Symbol(">")) => ComparisonOperator::Gt,
            Some(Token::Symbol(">=")) => ComparisonOperator::Gte,
            _ => {
                self.position = self.position.saturating_sub(1);
                return Err(self.syntax("expected a comparison operator"));
            }
        };
        let right = self.expr()?;
        Ok(Condition::Comparison {
            left,
            operator,
            right,
        })
    }

    // -- INSERT / UPDATE / DELETE -----------------------------------------------

    fn insert(&mut self) -> Result<SqlCommand, SqlError> {
        self.expect_keyword("INSERT")?;
        self.expect_keyword("INTO")?;
        let table = self.table_ref()?;
        if is_relationship_table(&table.name) {
            return Err(unsupported(
                "inserting relationships through SQL; use Cypher `CREATE (a)-[r:TYPE]->(b)`",
            ));
        }
        self.expect_symbol("(")?;
        let mut columns = Vec::new();
        loop {
            columns.push(self.identifier()?);
            if self.eat_symbol(")") {
                break;
            }
            self.expect_symbol(",")?;
        }
        if columns
            .iter()
            .any(|column| column == "source_id" || column == "target_id")
        {
            return Err(unsupported(
                "inserting relationships through SQL; use Cypher `CREATE (a)-[r:TYPE]->(b)`",
            ));
        }
        for column in &columns {
            if is_structural_column(column) {
                return Err(error(
                    sqlstate::UNDEFINED_COLUMN,
                    format!("`{column}` is derived by the graph and cannot be inserted"),
                ));
            }
        }
        if self.eat_keyword("SELECT") {
            return Err(unsupported("INSERT ... SELECT"));
        }
        self.expect_keyword("VALUES")?;
        self.expect_symbol("(")?;
        let mut values = Vec::new();
        loop {
            match self.expr()? {
                Expr::Literal(value) => values.push(value),
                _ => return Err(self.syntax("VALUES take literals or parameters")),
            }
            if self.eat_symbol(")") {
                break;
            }
            self.expect_symbol(",")?;
        }
        if self.eat_symbol(",") {
            return Err(unsupported("multi-row VALUES; issue one INSERT per row"));
        }
        if values.len() != columns.len() {
            return Err(
                self.syntax("INSERT has more target columns than expressions or the reverse")
            );
        }
        let returning = if self.eat_keyword("RETURNING") {
            Some(self.select_items()?)
        } else {
            None
        };
        if self.eat_keyword("ON") {
            return Err(unsupported("ON CONFLICT"));
        }
        self.expect_end()?;
        let variable = if table.alias == table.name {
            generated_variable(&table.name)
        } else {
            table.alias.clone()
        };
        let tables = vec![TableRef {
            name: table.name.clone(),
            alias: variable.clone(),
        }];
        let scope = Scope::new(&tables, &[])?;
        let properties = columns.into_iter().zip(values).collect();
        let mut query = ParsedQuery {
            match_clause: None,
            where_clause: None,
            return_clause: None,
            create_clause: Some(CreateClause {
                nodes: vec![NodePattern {
                    variable: variable.clone(),
                    label: label_for_table(&table.name),
                    properties,
                }],
                relationship: None,
            }),
            merge_clause: None,
            set_clause: None,
            delete_clause: None,
            remove_clause: None,
        };
        let mut output_columns = Vec::new();
        let mut post = PostProcess::default();
        if let Some(items) = returning {
            let projection = Projection::plan(&scope, &items, false)?;
            query.return_clause = Some(ReturnClause {
                distinct: false,
                items: projection.items,
                order_by: vec![],
                skip: None,
                limit: None,
            });
            output_columns = projection.columns;
            post = projection.post;
        }
        Ok(SqlCommand::Query(CompiledQuery {
            ast: QueryAst::from_structured(self.source, query),
            mode: QueryMode::Mutation,
            columns: output_columns,
            tag: CommandTag::Insert,
            post,
        }))
    }

    fn update(&mut self) -> Result<SqlCommand, SqlError> {
        self.expect_keyword("UPDATE")?;
        let table = self.table_ref()?;
        let table = with_variable(table);
        self.expect_keyword("SET")?;
        let mut assignments = Vec::new();
        loop {
            let target = match self.expr()? {
                Expr::Column { qualifier, name } => {
                    if let Some(qualifier) = qualifier
                        && qualifier != table.alias
                    {
                        return Err(error(
                            sqlstate::UNDEFINED_TABLE,
                            format!("unknown table alias `{qualifier}`"),
                        ));
                    }
                    if is_structural_column(&name) {
                        return Err(error(
                            sqlstate::UNDEFINED_COLUMN,
                            format!("`{name}` is derived by the graph and cannot be updated"),
                        ));
                    }
                    name
                }
                _ => return Err(self.syntax("expected a column to assign")),
            };
            self.expect_symbol("=")?;
            let value = match self.expr()? {
                Expr::Literal(value) => value,
                _ => {
                    return Err(unsupported(
                        "computed values in SET; assign literals or parameters",
                    ));
                }
            };
            assignments.push(SetAssignment {
                target: PropertyRef {
                    variable: table.alias.clone(),
                    property: target,
                },
                value,
            });
            if !self.eat_symbol(",") {
                break;
            }
        }
        if self.eat_keyword("FROM") {
            return Err(unsupported("UPDATE ... FROM"));
        }
        let condition = if self.eat_keyword("WHERE") {
            Some(self.condition()?)
        } else {
            None
        };
        if self.eat_keyword("RETURNING") {
            return Err(unsupported(
                "RETURNING on UPDATE; read the rows back with SELECT",
            ));
        }
        self.expect_end()?;
        let tables = vec![table.clone()];
        let scope = Scope::new(&tables, &[])?;
        let where_clause = condition
            .map(|condition| compile_condition(&scope, condition))
            .transpose()?;
        let query = ParsedQuery {
            match_clause: Some(MatchClause {
                optional: false,
                start: NodePattern {
                    variable: table.alias.clone(),
                    label: label_for_table(&table.name),
                    properties: vec![],
                },
                relationship: None,
                additional_nodes: vec![],
            }),
            where_clause: where_clause.map(|expression| WhereClause { expression }),
            return_clause: None,
            create_clause: None,
            merge_clause: None,
            set_clause: Some(SetClause { assignments }),
            delete_clause: None,
            remove_clause: None,
        };
        Ok(SqlCommand::Query(CompiledQuery {
            ast: QueryAst::from_structured(self.source, query),
            mode: QueryMode::Mutation,
            columns: vec![],
            tag: CommandTag::Update,
            post: PostProcess::default(),
        }))
    }

    fn delete(&mut self) -> Result<SqlCommand, SqlError> {
        self.expect_keyword("DELETE")?;
        self.expect_keyword("FROM")?;
        let table = with_variable(self.table_ref()?);
        if self.eat_keyword("USING") {
            return Err(unsupported("DELETE ... USING"));
        }
        let condition = if self.eat_keyword("WHERE") {
            Some(self.condition()?)
        } else {
            None
        };
        if self.eat_keyword("RETURNING") {
            return Err(unsupported("RETURNING on DELETE"));
        }
        self.expect_end()?;
        let tables = vec![table.clone()];
        let scope = Scope::new(&tables, &[])?;
        let where_clause = condition
            .map(|condition| compile_condition(&scope, condition))
            .transpose()?;
        let query = ParsedQuery {
            match_clause: Some(MatchClause {
                optional: false,
                start: NodePattern {
                    variable: table.alias.clone(),
                    label: label_for_table(&table.name),
                    properties: vec![],
                },
                relationship: None,
                additional_nodes: vec![],
            }),
            where_clause: where_clause.map(|expression| WhereClause { expression }),
            return_clause: None,
            create_clause: None,
            merge_clause: None,
            set_clause: None,
            delete_clause: Some(DeleteClause {
                variables: vec![table.alias.clone()],
            }),
            remove_clause: None,
        };
        Ok(SqlCommand::Query(CompiledQuery {
            ast: QueryAst::from_structured(self.source, query),
            mode: QueryMode::Mutation,
            columns: vec![],
            tag: CommandTag::Delete,
            post: PostProcess::default(),
        }))
    }

    // -- scalar and catalog ----------------------------------------------------

    fn scalar(&self, items: Vec<SelectItem>) -> Result<SqlCommand, SqlError> {
        let mut columns = Vec::new();
        let mut row = Vec::new();
        for item in items {
            let (name, cell) = match &item.expr {
                Expr::Literal(value) => ("?column?".to_owned(), literal_cell(value)),
                Expr::Function(function) => (
                    function.clone(),
                    match function.as_str() {
                        "version" => Cell::Text(format!("PostgreSQL {SERVER_VERSION}")),
                        "current_user" | "session_user" | "current_role" | "user" => {
                            Cell::Text("corrobore".to_owned())
                        }
                        "current_database" | "current_catalog" => {
                            Cell::Text("corrobore".to_owned())
                        }
                        "current_schema" => Cell::Text("public".to_owned()),
                        "now"
                        | "current_timestamp"
                        | "transaction_timestamp"
                        | "statement_timestamp" => {
                            return Err(unsupported("temporal functions"));
                        }
                        other => {
                            return Err(error(
                                sqlstate::UNDEFINED_COLUMN,
                                format!("unknown function `{other}`"),
                            ));
                        }
                    },
                ),
                Expr::Star => return Err(self.syntax("`*` needs a FROM clause")),
                Expr::Aggregate { .. } => return Err(self.syntax("aggregates need a FROM clause")),
                Expr::Column { name, .. } => {
                    return Err(error(
                        sqlstate::UNDEFINED_COLUMN,
                        format!("column `{name}` does not exist without a FROM clause"),
                    ));
                }
            };
            columns.push(OutputColumn {
                name: item.alias.unwrap_or(name),
                source: ColumnSource::Constant(cell.clone()),
            });
            row.push(cell);
        }
        Ok(SqlCommand::Scalar(ScalarResult { columns, row }))
    }

    fn catalog(
        &mut self,
        kind: CatalogKind,
        items: Vec<SelectItem>,
    ) -> Result<SqlCommand, SqlError> {
        // An optional alias for the view.
        if let Some(Token::Ident { text, .. }) = self.peek().cloned() {
            if text.eq_ignore_ascii_case("AS") {
                self.position += 1;
                self.identifier()?;
            } else if !is_reserved(&text) {
                self.position += 1;
            }
        }
        let mut table = None;
        if self.eat_keyword("WHERE") {
            let condition = self.condition()?;
            let mut conjuncts = Vec::new();
            flatten_and(condition, &mut conjuncts);
            for conjunct in conjuncts {
                match conjunct {
                    Condition::Comparison {
                        left: Expr::Column { name, .. },
                        operator: ComparisonOperator::Eq,
                        right: Expr::Literal(LiteralValue::String(value)),
                    } => match name.to_ascii_lowercase().as_str() {
                        "table_name" => table = Some(value),
                        "table_schema" | "table_catalog" => {
                            if value != "public" && value != "corrobore" {
                                // Another schema holds nothing: an impossible
                                // filter, expressed as a table that does not exist.
                                table = Some(String::from("\u{0}"));
                            }
                        }
                        "table_type" => {}
                        other => {
                            return Err(unsupported(&format!(
                                "information_schema filter on `{other}`"
                            )));
                        }
                    },
                    Condition::In {
                        left: Expr::Column { name, .. },
                        ..
                    } if name.eq_ignore_ascii_case("table_schema")
                        || name.eq_ignore_ascii_case("table_type") => {}
                    // A NULL literal is an unbound placeholder from a Parse-time
                    // syntax check: the filter is decided once Bind supplies it.
                    Condition::Comparison {
                        left: Expr::Column { .. },
                        operator: ComparisonOperator::Eq,
                        right: Expr::Literal(LiteralValue::Null),
                    } => {}
                    _ => return Err(unsupported("this information_schema filter shape")),
                }
            }
        }
        let mut order_by = None;
        if self.eat_keyword("ORDER") {
            self.expect_keyword("BY")?;
            match self.expr()? {
                Expr::Column { name, .. } => order_by = Some(name.to_ascii_lowercase()),
                _ => return Err(self.syntax("ORDER BY a catalog column")),
            }
            if !self.eat_keyword("ASC") {
                self.eat_keyword("DESC");
            }
        }
        if self.eat_keyword("LIMIT") {
            self.count()?;
        }
        self.expect_end()?;
        let mut columns = Vec::new();
        for item in items {
            match item.expr {
                Expr::Star => {
                    columns.clear();
                    break;
                }
                Expr::Column { name, .. } => {
                    columns.push(item.alias.unwrap_or(name).to_ascii_lowercase())
                }
                _ => return Err(unsupported("expressions over information_schema")),
            }
        }
        Ok(SqlCommand::Catalog(CatalogQuery {
            kind,
            table,
            columns,
            order_by,
        }))
    }
}

fn flatten_and(condition: Condition, out: &mut Vec<Condition>) {
    match condition {
        Condition::And(terms) => {
            for term in terms {
                flatten_and(term, out);
            }
        }
        other => out.push(other),
    }
}

fn number_literal(text: &str) -> Result<LiteralValue, SqlError> {
    if text.contains(['.', 'e', 'E']) {
        text.parse::<f64>()
            .map_err(|_| error(sqlstate::SYNTAX_ERROR, format!("invalid number `{text}`")))?;
        Ok(LiteralValue::Float(text.to_owned()))
    } else {
        text.parse::<i64>().map(LiteralValue::Integer).map_err(|_| {
            error(
                sqlstate::SYNTAX_ERROR,
                format!("integer `{text}` is out of range"),
            )
        })
    }
}

fn literal_cell(value: &LiteralValue) -> Cell {
    match value {
        LiteralValue::Null => Cell::Null,
        LiteralValue::Boolean(value) => Cell::Bool(*value),
        LiteralValue::Integer(value) => Cell::Int(*value),
        LiteralValue::Float(text) => Cell::Float(text.clone()),
        LiteralValue::String(text) => Cell::Text(text.clone()),
        LiteralValue::List(values) => Cell::Json(serde_json::Value::Array(
            values
                .iter()
                .map(|value| cell_to_json(&literal_cell(value)))
                .collect(),
        )),
        LiteralValue::PropertyReferenceList(_) => Cell::Null,
    }
}

const RESERVED: [&str; 26] = [
    "FROM",
    "WHERE",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "CROSS",
    "ON",
    "GROUP",
    "ORDER",
    "LIMIT",
    "OFFSET",
    "HAVING",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "AS",
    "SET",
    "VALUES",
    "RETURNING",
    "FOR",
    "FETCH",
    "USING",
    "AND",
    "OR",
];

fn is_reserved(word: &str) -> bool {
    RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(word))
}

fn is_relationship_table(name: &str) -> bool {
    name == "relationships"
}

fn is_structural_column(name: &str) -> bool {
    matches!(
        name,
        "id" | "labels"
            | "type"
            | "source_id"
            | "target_id"
            | "status"
            | "confidence"
            | "evidence_refs"
            | "properties"
    )
}

/// `nodes` scans every label; any other table name is a label.
fn label_for_table(name: &str) -> Option<String> {
    if name == "nodes" {
        None
    } else {
        Some(name.to_owned())
    }
}

/// A Cypher-safe variable for a table that has no alias.
fn generated_variable(table: &str) -> String {
    let mut variable = String::from("t_");
    variable.extend(
        table
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '_'),
    );
    variable
}

/// A table without an explicit alias gets a generated Cypher variable, so a
/// quoted label such as `"Threat Actor"` never has to be a variable name.
fn with_variable(table: TableRef) -> TableRef {
    if table.alias == table.name {
        TableRef {
            alias: generated_variable(&table.name),
            ..table
        }
    } else {
        table
    }
}

// ---------------------------------------------------------------------------
// Scope: which alias is a node, which is the relationship, in which direction
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Scope {
    /// Alias of the pattern's start node.
    source: TableRef,
    /// Relationship alias and the target node, when the shape has one.
    relationship: Option<(TableRef, TableRef)>,
    /// `relationships` scans need a type filter folded into the pattern.
    relationship_type: Option<String>,
}

impl Scope {
    fn new(tables: &[TableRef], joins: &[(Expr, Expr)]) -> Result<Self, SqlError> {
        let alias_of = |expr: &Expr| -> Result<(String, String), SqlError> {
            match expr {
                Expr::Column {
                    qualifier: Some(qualifier),
                    name,
                } => Ok((qualifier.clone(), name.clone())),
                Expr::Column {
                    qualifier: None,
                    name,
                } => Err(error(
                    sqlstate::AMBIGUOUS_COLUMN,
                    format!("join column `{name}` must be qualified with its table alias"),
                )),
                _ => Err(error(
                    sqlstate::SYNTAX_ERROR,
                    "join conditions compare two columns",
                )),
            }
        };
        let find = |alias: &str| -> Result<TableRef, SqlError> {
            tables
                .iter()
                .find(|table| table.alias == alias)
                .cloned()
                .ok_or_else(|| {
                    error(
                        sqlstate::UNDEFINED_TABLE,
                        format!("unknown table alias `{alias}`"),
                    )
                })
        };
        match tables.len() {
            1 => {
                let only = tables[0].clone();
                if is_relationship_table(&only.name) {
                    let hidden = |suffix: &str| TableRef {
                        name: "nodes".to_owned(),
                        alias: format!("__{}_{suffix}", only.alias),
                    };
                    let source = hidden("source");
                    let target = hidden("target");
                    Ok(Self {
                        source,
                        relationship: Some((only, target)),
                        relationship_type: None,
                    })
                } else {
                    Ok(Self {
                        source: only,
                        relationship: None,
                        relationship_type: None,
                    })
                }
            }
            2 | 3 => {
                let mut source: Option<String> = None;
                let mut target: Option<String> = None;
                let mut relationship: Option<String> = None;
                for (left, right) in joins {
                    let (left_alias, left_column) = alias_of(left)?;
                    let (right_alias, right_column) = alias_of(right)?;
                    let (rel_alias, endpoint, node_alias, node_column) = if left_column
                        == "source_id"
                        || left_column == "target_id"
                    {
                        (left_alias, left_column, right_alias, right_column)
                    } else if right_column == "source_id" || right_column == "target_id" {
                        (right_alias, right_column, left_alias, left_column)
                    } else {
                        return Err(unsupported(
                            "joins that are not `relationship.source_id = node.id` or `relationship.target_id = node.id`",
                        ));
                    };
                    if node_column != "id" {
                        return Err(unsupported("joining on a column other than `id`"));
                    }
                    if relationship
                        .as_deref()
                        .is_some_and(|known| known != rel_alias)
                    {
                        return Err(unsupported("more than one relationship in a query"));
                    }
                    relationship = Some(rel_alias);
                    let slot = if endpoint == "source_id" {
                        &mut source
                    } else {
                        &mut target
                    };
                    if slot.is_some() {
                        return Err(unsupported("joining the same endpoint twice"));
                    }
                    *slot = Some(node_alias);
                }
                let Some(relationship) = relationship else {
                    return Err(unsupported(
                        "node-to-node joins without a relationship table",
                    ));
                };
                let relationship = find(&relationship)?;
                let hidden = |suffix: &str| TableRef {
                    name: "nodes".to_owned(),
                    alias: format!("__{}_{suffix}", relationship.alias),
                };
                let source = match source {
                    Some(alias) => find(&alias)?,
                    None => hidden("source"),
                };
                let target = match target {
                    Some(alias) => find(&alias)?,
                    None => hidden("target"),
                };
                // Every table must play a role.
                for table in tables {
                    if table.alias != source.alias
                        && table.alias != target.alias
                        && table.alias != relationship.alias
                    {
                        return Err(unsupported(&format!(
                            "table `{}` is not connected by a join",
                            table.alias
                        )));
                    }
                }
                let relationship_type = if is_relationship_table(&relationship.name) {
                    None
                } else {
                    Some(relationship.name.clone())
                };
                Ok(Self {
                    source,
                    relationship: Some((relationship, target)),
                    relationship_type,
                })
            }
            _ => Err(unsupported("more than one relationship in a query")),
        }
    }

    fn is_relationship_alias(&self, alias: &str) -> bool {
        self.relationship
            .as_ref()
            .is_some_and(|(rel, _)| rel.alias == alias)
    }

    fn is_known_alias(&self, alias: &str) -> bool {
        self.source.alias == alias
            || self
                .relationship
                .as_ref()
                .is_some_and(|(rel, target)| rel.alias == alias || target.alias == alias)
    }

    /// Visible aliases in pattern order (source, relationship, target).
    fn visible_aliases(&self) -> Vec<String> {
        let mut aliases = Vec::new();
        if !self.source.alias.starts_with("__") {
            aliases.push(self.source.alias.clone());
        }
        if let Some((rel, target)) = &self.relationship {
            aliases.push(rel.alias.clone());
            if !target.alias.starts_with("__") {
                aliases.push(target.alias.clone());
            }
        }
        aliases
    }

    /// Resolve a possibly unqualified column to its alias.
    fn resolve(&self, qualifier: Option<&str>, name: &str) -> Result<String, SqlError> {
        match qualifier {
            Some(alias) => {
                if self.is_known_alias(alias) {
                    Ok(alias.to_owned())
                } else {
                    Err(error(
                        sqlstate::UNDEFINED_TABLE,
                        format!("unknown table alias `{alias}`"),
                    ))
                }
            }
            None => {
                let visible = self.visible_aliases();
                if visible.len() == 1 {
                    Ok(visible[0].clone())
                } else {
                    Err(error(
                        sqlstate::AMBIGUOUS_COLUMN,
                        format!("column `{name}` must be qualified when several tables are joined"),
                    ))
                }
            }
        }
    }

    fn match_clause(&self, relationship_type: Option<String>) -> MatchClause {
        MatchClause {
            optional: false,
            start: NodePattern {
                variable: self.source.alias.clone(),
                label: label_for_table(&self.source.name),
                properties: vec![],
            },
            relationship: self.relationship.as_ref().map(|(rel, target)| {
                (
                    RelationshipPattern {
                        variable: Some(rel.alias.clone()),
                        rel_type: relationship_type.or_else(|| self.relationship_type.clone()),
                    },
                    NodePattern {
                        variable: target.alias.clone(),
                        label: label_for_table(&target.name),
                        properties: vec![],
                    },
                )
            }),
            additional_nodes: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Projection planning
// ---------------------------------------------------------------------------

struct Projection {
    items: Vec<ProjectionItem>,
    columns: Vec<OutputColumn>,
    post: PostProcess,
    /// Whole variables are returned and cells derived afterwards.
    whole_records: bool,
    aggregated: bool,
}

impl Projection {
    fn plan(scope: &Scope, items: &[SelectItem], distinct: bool) -> Result<Self, SqlError> {
        let mut aggregates = 0;
        let mut plain = 0;
        let mut needs_whole = false;
        for item in items {
            match &item.expr {
                Expr::Aggregate { .. } => aggregates += 1,
                Expr::Star => {
                    plain += 1;
                    needs_whole = true;
                }
                Expr::Column { qualifier, name } => {
                    plain += 1;
                    let alias = scope.resolve(qualifier.as_deref(), name)?;
                    if entity_field(scope, &alias, name)
                        .is_some_and(|field| !field_is_property_ref(&field))
                    {
                        needs_whole = true;
                    }
                }
                Expr::Literal(_) | Expr::Function(_) => {
                    return Err(unsupported(
                        "constant or function columns beside table columns",
                    ));
                }
            }
        }
        if aggregates > 0 && plain > 0 {
            return Err(error(
                sqlstate::GROUPING_ERROR,
                "aggregate and non-aggregate columns cannot be mixed without GROUP BY, which is not supported",
            ));
        }
        let mut projection_items = Vec::new();
        let mut columns = Vec::new();
        if aggregates > 0 {
            for item in items {
                let Expr::Aggregate { function, argument } = &item.expr else {
                    unreachable!("mixed projections were refused above");
                };
                let (projection, key) = aggregate_item(scope, function, argument.as_deref())?;
                projection_items.push(projection);
                columns.push(OutputColumn {
                    name: item.alias.clone().unwrap_or_else(|| function.clone()),
                    source: ColumnSource::Field(key),
                });
            }
            return Ok(Self {
                items: projection_items,
                columns,
                post: PostProcess::default(),
                whole_records: false,
                aggregated: true,
            });
        }
        if needs_whole {
            let mut returned: Vec<String> = Vec::new();
            for item in items {
                match &item.expr {
                    Expr::Star => {
                        for alias in scope.visible_aliases() {
                            if !returned.contains(&alias) {
                                returned.push(alias.clone());
                            }
                            let is_relationship = scope.is_relationship_alias(&alias);
                            let fields: &[(&str, EntityField)] = if is_relationship {
                                &[
                                    ("id", EntityField::Id),
                                    ("type", EntityField::Type),
                                    ("source_id", EntityField::SourceId),
                                    ("target_id", EntityField::TargetId),
                                    ("status", EntityField::Status),
                                    ("confidence", EntityField::Confidence),
                                    ("evidence_refs", EntityField::EvidenceRefs),
                                    ("properties", EntityField::Properties),
                                ]
                            } else {
                                &[
                                    ("id", EntityField::Id),
                                    ("labels", EntityField::Labels),
                                    ("status", EntityField::Status),
                                    ("confidence", EntityField::Confidence),
                                    ("evidence_refs", EntityField::EvidenceRefs),
                                    ("properties", EntityField::Properties),
                                ]
                            };
                            for (name, field) in fields {
                                columns.push(OutputColumn {
                                    name: (*name).to_owned(),
                                    source: ColumnSource::Entity {
                                        variable: alias.clone(),
                                        field: field.clone(),
                                    },
                                });
                            }
                        }
                    }
                    Expr::Column { qualifier, name } => {
                        let alias = scope.resolve(qualifier.as_deref(), name)?;
                        if !returned.contains(&alias) {
                            returned.push(alias.clone());
                        }
                        let field = entity_field(scope, &alias, name)
                            .unwrap_or_else(|| EntityField::Property(name.clone()));
                        columns.push(OutputColumn {
                            name: item.alias.clone().unwrap_or_else(|| name.clone()),
                            source: ColumnSource::Entity {
                                variable: alias,
                                field,
                            },
                        });
                    }
                    _ => unreachable!("checked above"),
                }
            }
            return Ok(Self {
                items: returned.into_iter().map(ProjectionItem::Variable).collect(),
                columns,
                post: PostProcess {
                    distinct,
                    ..PostProcess::default()
                },
                whole_records: true,
                aggregated: false,
            });
        }
        for item in items {
            let Expr::Column { qualifier, name } = &item.expr else {
                unreachable!("checked above");
            };
            let alias = scope.resolve(qualifier.as_deref(), name)?;
            let property = PropertyRef {
                variable: alias.clone(),
                property: name.clone(),
            };
            projection_items.push(ProjectionItem::Property(property));
            columns.push(OutputColumn {
                name: item.alias.clone().unwrap_or_else(|| name.clone()),
                source: ColumnSource::Field(format!("{alias}.{name}")),
            });
        }
        Ok(Self {
            items: projection_items,
            columns,
            post: PostProcess::default(),
            whole_records: false,
            aggregated: false,
        })
    }
}

/// Structural meaning of a column name for an alias, if any.
fn entity_field(scope: &Scope, alias: &str, name: &str) -> Option<EntityField> {
    let is_relationship = scope.is_relationship_alias(alias);
    Some(match name {
        "id" => EntityField::Id,
        "labels" if !is_relationship => EntityField::Labels,
        "type" if is_relationship => EntityField::Type,
        "source_id" if is_relationship => EntityField::SourceId,
        "target_id" if is_relationship => EntityField::TargetId,
        "status" => EntityField::Status,
        "confidence" => EntityField::Confidence,
        "evidence_refs" => EntityField::EvidenceRefs,
        "properties" => EntityField::Properties,
        _ => return None,
    })
}

/// Fields the executor already exposes through property access, so they can
/// be pushed down as property projections and predicates.
fn field_is_property_ref(field: &EntityField) -> bool {
    matches!(
        field,
        EntityField::Id | EntityField::Status | EntityField::Confidence | EntityField::Property(_)
    )
}

fn aggregate_item(
    scope: &Scope,
    function: &str,
    argument: Option<&Expr>,
) -> Result<(ProjectionItem, String), SqlError> {
    match (function, argument) {
        ("count", None) => Ok((ProjectionItem::Count("*".to_owned()), "count".to_owned())),
        ("count", Some(Expr::Star)) => {
            Ok((ProjectionItem::Count("*".to_owned()), "count".to_owned()))
        }
        ("count", Some(Expr::Column { qualifier, name })) => {
            let alias = match qualifier {
                // count(alias) counts rows binding the alias; count(alias.col)
                // is refused because the executor counts bindings, not values.
                None if scope.is_known_alias(name) => name.clone(),
                None => scope.resolve(None, name)?,
                Some(_) => {
                    return Err(unsupported(
                        "count(alias.column); use count(*) or count(alias)",
                    ));
                }
            };
            Ok((ProjectionItem::Count(alias), "count".to_owned()))
        }
        ("sum" | "avg" | "min" | "max", Some(Expr::Column { qualifier, name })) => {
            let alias = scope.resolve(qualifier.as_deref(), name)?;
            let property = PropertyRef {
                variable: alias.clone(),
                property: name.clone(),
            };
            let key = format!("{function}({alias}.{name})");
            let item = match function {
                "sum" => ProjectionItem::Sum(property),
                "avg" => ProjectionItem::Average(property),
                "min" => ProjectionItem::Minimum(property),
                _ => ProjectionItem::Maximum(property),
            };
            Ok((item, key))
        }
        _ => Err(unsupported(&format!("this form of {function}()"))),
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_select(
    source: &str,
    scope: &Scope,
    distinct: bool,
    items: Vec<SelectItem>,
    condition: Option<Condition>,
    order_by: Vec<(Expr, OrderDirection)>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<SqlCommand, SqlError> {
    // A `relationship.type = 'X'` conjunct folds into the pattern.
    let mut relationship_type = None;
    let condition = match condition {
        Some(condition) => {
            let mut conjuncts = Vec::new();
            flatten_and(condition, &mut conjuncts);
            let mut kept = Vec::new();
            for conjunct in conjuncts {
                if let Condition::Comparison {
                    left: Expr::Column { qualifier, name },
                    operator: ComparisonOperator::Eq,
                    right: Expr::Literal(LiteralValue::String(value)),
                } = &conjunct
                    && name == "type"
                    && qualifier
                        .as_deref()
                        .is_some_and(|alias| scope.is_relationship_alias(alias))
                    && scope.relationship_type.is_none()
                    && relationship_type.is_none()
                {
                    relationship_type = Some(value.clone());
                    continue;
                }
                kept.push(conjunct);
            }
            match kept.len() {
                0 => None,
                1 => Some(kept.remove(0)),
                _ => Some(Condition::And(kept)),
            }
        }
        None => None,
    };
    let where_clause = condition
        .map(|condition| compile_condition(scope, condition))
        .transpose()?
        .map(|expression| WhereClause { expression });

    let projection = Projection::plan(scope, &items, distinct)?;
    let mut order = Vec::new();
    for (expr, direction) in order_by {
        let Expr::Column { qualifier, name } = expr else {
            return Err(unsupported(
                "ORDER BY an expression or aggregate; order by a column",
            ));
        };
        let alias = scope.resolve(qualifier.as_deref(), &name)?;
        if entity_field(scope, &alias, &name).is_some_and(|field| !field_is_property_ref(&field)) {
            return Err(unsupported(&format!(
                "ORDER BY the structural column `{name}`"
            )));
        }
        order.push(OrderBy {
            field: PropertyRef {
                variable: alias,
                property: name,
            },
            direction,
        });
    }
    if projection.aggregated && !order.is_empty() {
        return Err(unsupported("ORDER BY with aggregates"));
    }
    // Whole-record rows are one record each, so OFFSET and LIMIT push down
    // unchanged. DISTINCT is the exception: it applies to the projected cells,
    // not to the records, so it and its bounds run after projection.
    let (post, skip, pushed_limit, pushed_distinct) = if projection.whole_records && distinct {
        (
            PostProcess {
                distinct: true,
                offset: offset.unwrap_or(0),
                limit,
            },
            None,
            None,
            false,
        )
    } else {
        (PostProcess::default(), offset, limit, distinct)
    };
    let query = ParsedQuery {
        match_clause: Some(scope.match_clause(relationship_type)),
        where_clause,
        return_clause: Some(ReturnClause {
            distinct: pushed_distinct,
            items: projection.items,
            order_by: order,
            skip,
            limit: pushed_limit,
        }),
        create_clause: None,
        merge_clause: None,
        set_clause: None,
        delete_clause: None,
        remove_clause: None,
    };
    Ok(SqlCommand::Query(CompiledQuery {
        ast: QueryAst::from_structured(source, query),
        mode: QueryMode::Read,
        columns: projection.columns,
        tag: CommandTag::Select,
        post,
    }))
}

fn compile_condition(scope: &Scope, condition: Condition) -> Result<WhereExpression, SqlError> {
    let property_ref = |expr: &Expr| -> Result<PropertyRef, SqlError> {
        let Expr::Column { qualifier, name } = expr else {
            return Err(error(
                sqlstate::SYNTAX_ERROR,
                "predicates compare a column with a value",
            ));
        };
        let alias = scope.resolve(qualifier.as_deref(), name)?;
        if let Some(field) = entity_field(scope, &alias, name)
            && !field_is_property_ref(&field)
        {
            return Err(unsupported(&format!(
                "predicates on the structural column `{name}`"
            )));
        }
        Ok(PropertyRef {
            variable: alias,
            property: name.clone(),
        })
    };
    Ok(match condition {
        Condition::Comparison {
            left,
            operator,
            right,
        } => match (&left, &right) {
            (Expr::Column { .. }, Expr::Literal(value)) => WhereExpression::Comparison {
                left: property_ref(&left)?,
                operator,
                right: value.clone(),
            },
            (Expr::Literal(value), Expr::Column { .. }) => WhereExpression::Comparison {
                left: property_ref(&right)?,
                operator: flip(operator),
                right: value.clone(),
            },
            (Expr::Column { .. }, Expr::Column { .. }) => {
                return Err(unsupported("comparing two columns in WHERE"));
            }
            _ => {
                return Err(error(
                    sqlstate::SYNTAX_ERROR,
                    "predicates compare a column with a value",
                ));
            }
        },
        Condition::In {
            left,
            values,
            negated,
        } => WhereExpression::In {
            left: property_ref(&left)?,
            values,
            negated,
        },
        Condition::IsNull { left, negated } => WhereExpression::Exists {
            left: property_ref(&left)?,
            exists: negated,
        },
        Condition::And(terms) => WhereExpression::And(
            terms
                .into_iter()
                .map(|term| compile_condition(scope, term))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Condition::Or(terms) => WhereExpression::Or(
            terms
                .into_iter()
                .map(|term| compile_condition(scope, term))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    })
}

fn flip(operator: ComparisonOperator) -> ComparisonOperator {
    match operator {
        ComparisonOperator::Lt => ComparisonOperator::Gt,
        ComparisonOperator::Lte => ComparisonOperator::Gte,
        ComparisonOperator::Gt => ComparisonOperator::Lt,
        ComparisonOperator::Gte => ComparisonOperator::Lte,
        other => other,
    }
}

/// Compile one SQL statement.
///
/// `parameters` are the values bound for `$1`.. and then for named parameters
/// in first-use order.
///
/// # Errors
/// [`SqlError`] with the SQLSTATE a client acts on.
pub fn compile(sql: &str, parameters: &[SqlValue]) -> Result<SqlCommand, SqlError> {
    let mut parser = Parser::new(sql, parameters)?;
    let command = parser.statement()?;
    Ok(command)
}

// ---------------------------------------------------------------------------
// Row projection
// ---------------------------------------------------------------------------

const RESERVED_PROPERTY_KEYS: [&str; 3] = ["status", "confidence", "evidence_refs"];

/// Derive the SQL cells of one record from the executor's typed values.
#[must_use]
pub fn project_row(columns: &[OutputColumn], values: &HashMap<String, RecordValue>) -> Vec<Cell> {
    columns
        .iter()
        .map(|column| match &column.source {
            ColumnSource::Constant(cell) => cell.clone(),
            ColumnSource::Field(key) => values.get(key).map_or(Cell::Null, record_value_cell),
            ColumnSource::Entity { variable, field } => match values.get(variable) {
                Some(RecordValue::Node(node)) => match field {
                    EntityField::Id => Cell::Text(node.id.clone()),
                    EntityField::Labels => Cell::Json(serde_json::Value::Array(
                        node.labels
                            .iter()
                            .cloned()
                            .map(serde_json::Value::String)
                            .collect(),
                    )),
                    EntityField::Type | EntityField::SourceId | EntityField::TargetId => Cell::Null,
                    EntityField::Status => property_cell(&node.properties, "status"),
                    EntityField::Confidence => property_cell(&node.properties, "confidence"),
                    EntityField::EvidenceRefs => evidence_refs_cell(&node.properties),
                    EntityField::Properties => properties_cell(&node.properties),
                    EntityField::Property(name) => property_cell(&node.properties, name),
                },
                Some(RecordValue::Relationship(relationship)) => match field {
                    EntityField::Id => Cell::Text(relationship.id.clone()),
                    EntityField::Type => Cell::Text(relationship.rel_type.clone()),
                    EntityField::SourceId => Cell::Text(relationship.source_id.clone()),
                    EntityField::TargetId => Cell::Text(relationship.target_id.clone()),
                    EntityField::Labels => Cell::Null,
                    EntityField::Status => property_cell(&relationship.properties, "status"),
                    EntityField::Confidence => {
                        property_cell(&relationship.properties, "confidence")
                    }
                    EntityField::EvidenceRefs => evidence_refs_cell(&relationship.properties),
                    EntityField::Properties => properties_cell(&relationship.properties),
                    EntityField::Property(name) => property_cell(&relationship.properties, name),
                },
                _ => Cell::Null,
            },
        })
        .collect()
}

fn property_cell(properties: &BTreeMap<String, RecordValue>, name: &str) -> Cell {
    properties.get(name).map_or(Cell::Null, record_value_cell)
}

fn evidence_refs_cell(properties: &BTreeMap<String, RecordValue>) -> Cell {
    match properties.get("evidence_refs") {
        Some(value) => record_value_cell(value),
        None => Cell::Json(serde_json::Value::Array(vec![])),
    }
}

fn properties_cell(properties: &BTreeMap<String, RecordValue>) -> Cell {
    Cell::Json(serde_json::Value::Object(
        properties
            .iter()
            .filter(|(key, _)| !RESERVED_PROPERTY_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), record_value_json(value)))
            .collect(),
    ))
}

/// One typed record value as a SQL cell.
#[must_use]
pub fn record_value_cell(value: &RecordValue) -> Cell {
    match value {
        RecordValue::Null => Cell::Null,
        RecordValue::Boolean(value) => Cell::Bool(*value),
        RecordValue::Integer(value) => Cell::Int(*value),
        RecordValue::Float(text) => Cell::Float(text.clone()),
        RecordValue::String(text) => Cell::Text(text.clone()),
        RecordValue::List(_)
        | RecordValue::Map(_)
        | RecordValue::Node(_)
        | RecordValue::Relationship(_) => Cell::Json(record_value_json(value)),
    }
}

fn record_value_json(value: &RecordValue) -> serde_json::Value {
    match value {
        RecordValue::Null => serde_json::Value::Null,
        RecordValue::Boolean(value) => serde_json::Value::Bool(*value),
        RecordValue::Integer(value) => serde_json::Value::from(*value),
        RecordValue::Float(text) => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(
                || serde_json::Value::String(text.clone()),
                serde_json::Value::Number,
            ),
        RecordValue::String(text) => serde_json::Value::String(text.clone()),
        RecordValue::List(items) => {
            serde_json::Value::Array(items.iter().map(record_value_json).collect())
        }
        RecordValue::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), record_value_json(value)))
                .collect(),
        ),
        RecordValue::Node(node) => serde_json::json!({
            "id": node.id,
            "labels": node.labels,
            "properties": serde_json::Value::Object(
                node.properties.iter().map(|(key, value)| (key.clone(), record_value_json(value))).collect(),
            ),
        }),
        RecordValue::Relationship(relationship) => serde_json::json!({
            "id": relationship.id,
            "type": relationship.rel_type,
            "source_id": relationship.source_id,
            "target_id": relationship.target_id,
            "properties": serde_json::Value::Object(
                relationship.properties.iter().map(|(key, value)| (key.clone(), record_value_json(value))).collect(),
            ),
        }),
    }
}

fn cell_to_json(cell: &Cell) -> serde_json::Value {
    match cell {
        Cell::Null => serde_json::Value::Null,
        Cell::Bool(value) => serde_json::Value::Bool(*value),
        Cell::Int(value) => serde_json::Value::from(*value),
        Cell::Float(text) => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(
                || serde_json::Value::String(text.clone()),
                serde_json::Value::Number,
            ),
        Cell::Text(text) => serde_json::Value::String(text.clone()),
        Cell::Json(value) => value.clone(),
    }
}

impl Cell {
    /// The text a PostgreSQL client receives for this cell, `None` for NULL.
    #[must_use]
    pub fn to_text(&self) -> Option<String> {
        match self {
            Self::Null => None,
            Self::Bool(true) => Some("t".to_owned()),
            Self::Bool(false) => Some("f".to_owned()),
            Self::Int(value) => Some(value.to_string()),
            Self::Float(text) | Self::Text(text) => Some(text.clone()),
            Self::Json(value) => Some(value.to_string()),
        }
    }
}

/// Infer the type of a column from the cells it holds: NULL is ignored,
/// mixed integers and decimals are `float8`, anything mixed is `text`.
#[must_use]
pub fn column_type(cells: &[Cell]) -> ColumnType {
    let mut inferred: Option<ColumnType> = None;
    for cell in cells {
        let candidate = match cell {
            Cell::Null => continue,
            Cell::Bool(_) => ColumnType::Bool,
            Cell::Int(_) => ColumnType::Int8,
            Cell::Float(_) => ColumnType::Float8,
            Cell::Text(_) => ColumnType::Text,
            Cell::Json(_) => ColumnType::Jsonb,
        };
        inferred = Some(match inferred {
            None => candidate,
            Some(known) if known == candidate => known,
            Some(ColumnType::Int8) if candidate == ColumnType::Float8 => ColumnType::Float8,
            Some(ColumnType::Float8) if candidate == ColumnType::Int8 => ColumnType::Float8,
            Some(_) => ColumnType::Text,
        });
    }
    inferred.unwrap_or(ColumnType::Text)
}

/// Apply the post-processing a compiled query could not push down.
#[must_use]
pub fn post_process(post: &PostProcess, mut rows: Vec<Vec<Cell>>) -> Vec<Vec<Cell>> {
    if post.distinct {
        let mut seen = std::collections::HashSet::new();
        rows.retain(|row| seen.insert(format!("{row:?}")));
    }
    let rows = rows.into_iter().skip(post.offset);
    match post.limit {
        Some(limit) => rows.take(limit).collect(),
        None => rows.collect(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn tokenizer_handles_quotes_parameters_and_operators() {
        let tokens =
            tokenize(r#"SELECT "a b".x, 'it''s', $1, $name <> -3.5e2 -- trailing"#).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident {
                    text: "SELECT".into(),
                    quoted: false
                },
                Token::Ident {
                    text: "a b".into(),
                    quoted: true
                },
                Token::Symbol("."),
                Token::Ident {
                    text: "x".into(),
                    quoted: false
                },
                Token::Symbol(","),
                Token::Str("it's".into()),
                Token::Symbol(","),
                Token::Param("1".into()),
                Token::Symbol(","),
                Token::Param("name".into()),
                Token::Symbol("<>"),
                Token::Number("-3.5e2".into()),
            ]
        );
    }

    #[test]
    fn parameter_count_reads_the_highest_positional_placeholder() {
        assert_eq!(
            parameter_count("SELECT a.x FROM t AS a WHERE a.y = $2 AND a.z = $1"),
            2
        );
        assert_eq!(parameter_count("SELECT 1"), 0);
    }
}
