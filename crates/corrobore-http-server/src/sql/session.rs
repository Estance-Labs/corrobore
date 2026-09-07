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
//! One SQL connection's protocol state and statement execution.
//!
//! Module boundary: this module knows what a frontend message means in the
//! current state and turns SQL into engine calls through the shared AST. It
//! reads no socket; the listener frames bytes and hands them here. Every graph
//! statement runs through `CorroboreEngine::execute_prepared_request`, so the
//! policy, budgets, persistence and audit path are the ones every adapter uses.
//!
//! Transactions are statement groups, as for Bolt: each statement is applied
//! atomically and durably on its own. `ROLLBACK` after an applied write ends
//! the group and says so in a warning notice, because a PostgreSQL client
//! must always be able to leave a transaction.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use corrobore_engine::{EngineRequest, EngineRequestMode};
use graph_core::PropertyValue;
use shared_runtime::{CypherResponse, CypherResponseData, CypherResponseStatus};
use sql_frontend::{
    CatalogKind, CatalogQuery, Cell, ColumnSource, ColumnType, CommandTag, CompiledQuery,
    EntityField, QueryMode, SERVER_VERSION, SqlCommand, SqlError, SqlValue, column_type, compile,
    parameter_count, post_process, project_row, split_statements,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::wire::{Backend, Cursor};
use crate::{
    app::AppState, auth::token_matches, lifecycle::LifecycleState, security::AuthenticationMode,
};

const WORKSPACE_ID: &str = "workspace--sql-default";

// PostgreSQL type OIDs a client may declare for parameters.
const OID_BOOL: i32 = 16;
const OID_INT8: i32 = 20;
const OID_INT2: i32 = 21;
const OID_INT4: i32 = 23;
const OID_TEXT: i32 = 25;
const OID_FLOAT4: i32 = 700;
const OID_FLOAT8: i32 = 701;
const OID_VARCHAR: i32 = 1043;
const OID_NUMERIC: i32 = 1700;

/// SQLSTATE codes the session emits beyond the frontend's.
mod sqlstate {
    pub const WARNING: &str = "01000";
    pub const ACTIVE_TRANSACTION: &str = "25001";
    pub const NO_ACTIVE_TRANSACTION: &str = "25P01";
    pub const IN_FAILED_TRANSACTION: &str = "25P02";
    pub const INVALID_PASSWORD: &str = "28P01";
    pub const PROTOCOL_VIOLATION: &str = "08P01";
    pub const INSUFFICIENT_PRIVILEGE: &str = "42501";
    pub const UNDEFINED_OBJECT: &str = "42704";
    pub const SYNTAX_ERROR: &str = "42601";
    pub const PROGRAM_LIMIT_EXCEEDED: &str = "54000";
    pub const QUERY_CANCELED: &str = "57014";
    pub const ADMIN_SHUTDOWN: &str = "57P01";
    pub const INTERNAL_ERROR: &str = "XX000";
    pub const INVALID_PREPARED_STATEMENT: &str = "26000";
    pub const INVALID_CURSOR: &str = "34000";
}

/// A statement failure surfaced as an `ErrorResponse`.
#[derive(Clone, Debug)]
pub(super) struct Failure {
    sqlstate: &'static str,
    message: String,
}

impl Failure {
    fn new(sqlstate: &'static str, message: impl Into<String>) -> Self {
        Self {
            sqlstate,
            message: message.into(),
        }
    }
}

impl From<SqlError> for Failure {
    fn from(error: SqlError) -> Self {
        Self {
            sqlstate: error.sqlstate,
            message: error.message,
        }
    }
}

/// An executed statement's result, buffered before it is written.
#[derive(Clone, Debug)]
struct Executed {
    columns: Vec<(String, ColumnType)>,
    rows: Vec<Vec<Option<String>>>,
    tag: String,
    notices: Vec<Backend>,
}

impl Executed {
    fn command(tag: &str) -> Self {
        Self {
            columns: vec![],
            rows: vec![],
            tag: tag.to_owned(),
            notices: vec![],
        }
    }

    fn with_notice(mut self, notice: Backend) -> Self {
        self.notices.push(notice);
        self
    }
}

#[derive(Clone, Debug)]
struct Prepared {
    sql: String,
    parameter_oids: Vec<i32>,
}

#[derive(Clone, Debug)]
struct Portal {
    statement: String,
    parameters: Vec<SqlValue>,
    /// Filled by the first Describe or Execute; later Executes resume it.
    result: Option<Executed>,
    cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transaction {
    None,
    Open { read_only: bool, wrote: bool },
    Failed,
}

pub(super) struct Session {
    state: Arc<AppState>,
    session_id: String,
    transaction: Transaction,
    statements: HashMap<String, Prepared>,
    portals: HashMap<String, Portal>,
    /// After an extended-protocol error, everything until Sync is skipped.
    skip_until_sync: bool,
}

impl Session {
    pub(super) fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            session_id: format!("session--sql-{}", Uuid::new_v4()),
            transaction: Transaction::None,
            statements: HashMap::new(),
            portals: HashMap::new(),
            skip_until_sync: false,
        }
    }

    /// Authenticate the startup parameters. `Ok(true)` when a password round
    /// is required first; the caller then passes the password message.
    pub(super) fn needs_password(&self) -> bool {
        self.state.config.auth_mode == AuthenticationMode::Required
    }

    pub(super) fn check_password(&self, password: &str) -> bool {
        match self.state.config.auth_token.as_deref() {
            Some(expected) => token_matches(password, expected),
            None => false,
        }
    }

    /// The messages that follow `AuthenticationOk`.
    pub(super) fn welcome(&self) -> Vec<Backend> {
        let process_id =
            i32::from_be_bytes(Uuid::new_v4().as_bytes()[..4].try_into().unwrap_or([0; 4])).abs();
        let secret_key =
            i32::from_be_bytes(Uuid::new_v4().as_bytes()[4..8].try_into().unwrap_or([0; 4]));
        info!(session_id = %self.session_id, "sql session ready");
        vec![
            Backend::parameter_status("server_version", SERVER_VERSION),
            Backend::parameter_status("server_encoding", "UTF8"),
            Backend::parameter_status("client_encoding", "UTF8"),
            Backend::parameter_status("DateStyle", "ISO, MDY"),
            Backend::parameter_status("integer_datetimes", "on"),
            Backend::parameter_status("standard_conforming_strings", "on"),
            Backend::parameter_status("TimeZone", "UTC"),
            Backend::parameter_status("is_superuser", "off"),
            Backend::parameter_status("session_authorization", "corrobore"),
            Backend::backend_key_data(process_id, secret_key),
            Backend::ready_for_query(self.ready_status()),
        ]
    }

    pub(super) fn invalid_password() -> Backend {
        Backend::error(
            "FATAL",
            sqlstate::INVALID_PASSWORD,
            "password authentication failed; use the Corrobore bearer token as the password",
        )
    }

    fn ready_status(&self) -> u8 {
        match self.transaction {
            Transaction::None => b'I',
            Transaction::Open { .. } => b'T',
            Transaction::Failed => b'E',
        }
    }

    /// Handle one frontend message. Returns the messages to write and whether
    /// the connection should close afterwards.
    pub(super) async fn handle(&mut self, tag: u8, body: Vec<u8>) -> (Vec<Backend>, bool) {
        match tag {
            b'Q' => {
                self.skip_until_sync = false;
                let sql = match Cursor::new(&body).cstring() {
                    Ok(sql) => sql,
                    Err(_) => return self.protocol_violation("malformed Query message"),
                };
                let mut out = self.simple_query(&sql).await;
                out.push(Backend::ready_for_query(self.ready_status()));
                (out, false)
            }
            b'X' => (vec![], true),
            b'S' => {
                self.skip_until_sync = false;
                // Sync ends an implicit transaction block; explicit blocks stay.
                (vec![Backend::ready_for_query(self.ready_status())], false)
            }
            b'H' => (vec![], false),
            b'P' | b'B' | b'D' | b'E' | b'C' => {
                if self.skip_until_sync {
                    return (vec![], false);
                }
                match self.extended(tag, &body).await {
                    Ok(messages) => (messages, false),
                    Err(failure) => {
                        self.skip_until_sync = true;
                        if matches!(self.transaction, Transaction::Open { .. }) {
                            self.transaction = Transaction::Failed;
                        }
                        (
                            vec![Backend::error("ERROR", failure.sqlstate, &failure.message)],
                            false,
                        )
                    }
                }
            }
            b'd' | b'c' | b'f' => self.protocol_violation("COPY is not supported"),
            b'F' => {
                self.protocol_violation("the fast-path function call interface is not supported")
            }
            other => {
                self.protocol_violation(&format!("unexpected message type {:?}", other as char))
            }
        }
    }

    fn protocol_violation(&mut self, message: &str) -> (Vec<Backend>, bool) {
        (
            vec![Backend::error(
                "FATAL",
                sqlstate::PROTOCOL_VIOLATION,
                message,
            )],
            true,
        )
    }

    // -- simple query protocol -------------------------------------------------

    async fn simple_query(&mut self, sql: &str) -> Vec<Backend> {
        let statements = split_statements(sql);
        if statements.is_empty() {
            return vec![Backend::empty_query()];
        }
        let mut out = Vec::new();
        for statement in statements {
            match self.run_sql(&statement, &[]).await {
                Ok(executed) => {
                    out.extend(executed.notices.iter().cloned());
                    if !executed.columns.is_empty() {
                        out.push(Backend::row_description(&executed.columns));
                        for row in &executed.rows {
                            out.push(Backend::data_row(row));
                        }
                    }
                    if executed.tag.is_empty() {
                        out.push(Backend::empty_query());
                    } else {
                        out.push(Backend::command_complete(&executed.tag));
                    }
                }
                Err(failure) => {
                    out.push(Backend::error("ERROR", failure.sqlstate, &failure.message));
                    if matches!(self.transaction, Transaction::Open { .. }) {
                        self.transaction = Transaction::Failed;
                    }
                    // The rest of the batch is skipped, as PostgreSQL does.
                    break;
                }
            }
        }
        out
    }

    // -- extended query protocol -----------------------------------------------

    async fn extended(&mut self, tag: u8, body: &[u8]) -> Result<Vec<Backend>, Failure> {
        let mut cursor = Cursor::new(body);
        let malformed = |_| {
            Failure::new(
                sqlstate::PROTOCOL_VIOLATION,
                "malformed extended-protocol message",
            )
        };
        match tag {
            b'P' => {
                let name = cursor.cstring().map_err(malformed)?;
                let sql = cursor.cstring().map_err(malformed)?;
                let count = cursor.i16().map_err(malformed)?;
                let mut parameter_oids = Vec::new();
                for _ in 0..count.max(0) {
                    parameter_oids.push(cursor.i32().map_err(malformed)?);
                }
                // Syntax is checked now so a bad statement fails at Parse, as a
                // client expects; parameters are placeholders until Bind.
                let placeholders =
                    vec![SqlValue::Null; parameter_count(&sql).max(parameter_oids.len())];
                compile(&sql, &placeholders)?;
                self.statements.insert(
                    name,
                    Prepared {
                        sql,
                        parameter_oids,
                    },
                );
                Ok(vec![Backend::parse_complete()])
            }
            b'B' => {
                let portal = cursor.cstring().map_err(malformed)?;
                let statement = cursor.cstring().map_err(malformed)?;
                let prepared = self.statements.get(&statement).cloned().ok_or_else(|| {
                    Failure::new(
                        sqlstate::INVALID_PREPARED_STATEMENT,
                        format!("prepared statement \"{statement}\" does not exist"),
                    )
                })?;
                let format_count = cursor.i16().map_err(malformed)?.max(0) as usize;
                let mut formats = Vec::new();
                for _ in 0..format_count {
                    formats.push(cursor.i16().map_err(malformed)?);
                }
                let value_count = cursor.i16().map_err(malformed)?.max(0) as usize;
                let expected = parameter_count(&prepared.sql).max(prepared.parameter_oids.len());
                if value_count != expected {
                    return Err(Failure::new(
                        sqlstate::PROTOCOL_VIOLATION,
                        format!(
                            "bind message supplies {value_count} parameters, but prepared statement \"{statement}\" requires {expected}"
                        ),
                    ));
                }
                let mut parameters = Vec::new();
                for index in 0..value_count {
                    let length = cursor.i32().map_err(malformed)?;
                    let format = match formats.len() {
                        0 => 0,
                        1 => formats[0],
                        _ => formats.get(index).copied().unwrap_or(0),
                    };
                    let oid = prepared.parameter_oids.get(index).copied().unwrap_or(0);
                    if length < 0 {
                        parameters.push(SqlValue::Null);
                        continue;
                    }
                    let bytes = cursor.take(length as usize).map_err(malformed)?;
                    parameters.push(decode_parameter(bytes, format, oid)?);
                }
                let result_format_count = cursor.i16().map_err(malformed)?.max(0) as usize;
                for _ in 0..result_format_count {
                    if cursor.i16().map_err(malformed)? != 0 {
                        return Err(Failure::new(
                            sqlstate::PROTOCOL_VIOLATION,
                            "binary result formats are not supported; request text results",
                        ));
                    }
                }
                self.portals.insert(
                    portal,
                    Portal {
                        statement,
                        parameters,
                        result: None,
                        cursor: 0,
                    },
                );
                Ok(vec![Backend::bind_complete()])
            }
            b'D' => {
                let kind = cursor.byte().map_err(malformed)?;
                let name = cursor.cstring().map_err(malformed)?;
                match kind {
                    b'S' => {
                        let prepared = self.statements.get(&name).cloned().ok_or_else(|| {
                            Failure::new(
                                sqlstate::INVALID_PREPARED_STATEMENT,
                                format!("prepared statement \"{name}\" does not exist"),
                            )
                        })?;
                        let count =
                            parameter_count(&prepared.sql).max(prepared.parameter_oids.len());
                        let oids: Vec<i32> = (0..count)
                            .map(|index| match prepared.parameter_oids.get(index) {
                                Some(oid) if *oid != 0 => *oid,
                                _ => OID_TEXT,
                            })
                            .collect();
                        let mut out = vec![Backend::parameter_description(&oids)];
                        let placeholders = vec![SqlValue::Null; count];
                        match self
                            .describe_statement(&prepared.sql, &placeholders)
                            .await?
                        {
                            Some(columns) => out.push(Backend::row_description(&columns)),
                            None => out.push(Backend::no_data()),
                        }
                        Ok(out)
                    }
                    b'P' => {
                        let executed = self.execute_portal(&name).await?;
                        Ok(vec![if executed.columns.is_empty() {
                            Backend::no_data()
                        } else {
                            Backend::row_description(&executed.columns)
                        }])
                    }
                    _ => Err(Failure::new(
                        sqlstate::PROTOCOL_VIOLATION,
                        "Describe kind must be S or P",
                    )),
                }
            }
            b'E' => {
                let name = cursor.cstring().map_err(malformed)?;
                let max_rows = cursor.i32().map_err(malformed)?.max(0) as usize;
                let executed = self.execute_portal(&name).await?;
                let portal = self
                    .portals
                    .get_mut(&name)
                    .expect("portal exists after execution");
                let mut out: Vec<Backend> = executed.notices.clone();
                let remaining = executed.rows.len().saturating_sub(portal.cursor);
                let take = if max_rows == 0 {
                    remaining
                } else {
                    max_rows.min(remaining)
                };
                for row in executed.rows.iter().skip(portal.cursor).take(take) {
                    out.push(Backend::data_row(row));
                }
                portal.cursor += take;
                if portal.cursor < executed.rows.len() {
                    out.push(Backend::portal_suspended());
                } else if executed.tag.is_empty() {
                    out.push(Backend::empty_query());
                } else {
                    out.push(Backend::command_complete(&executed.tag));
                }
                Ok(out)
            }
            b'C' => {
                let kind = cursor.byte().map_err(malformed)?;
                let name = cursor.cstring().map_err(malformed)?;
                match kind {
                    b'S' => {
                        self.statements.remove(&name);
                        self.portals.retain(|_, portal| portal.statement != name);
                    }
                    b'P' => {
                        self.portals.remove(&name);
                    }
                    _ => {
                        return Err(Failure::new(
                            sqlstate::PROTOCOL_VIOLATION,
                            "Close kind must be S or P",
                        ));
                    }
                }
                Ok(vec![Backend::close_complete()])
            }
            _ => unreachable!("dispatched by tag"),
        }
    }

    async fn execute_portal(&mut self, name: &str) -> Result<Executed, Failure> {
        let portal = self.portals.get(name).cloned().ok_or_else(|| {
            Failure::new(
                sqlstate::INVALID_CURSOR,
                format!("portal \"{name}\" does not exist"),
            )
        })?;
        if let Some(result) = portal.result {
            return Ok(result);
        }
        let prepared = self
            .statements
            .get(&portal.statement)
            .cloned()
            .ok_or_else(|| {
                Failure::new(
                    sqlstate::INVALID_PREPARED_STATEMENT,
                    format!("prepared statement \"{}\" does not exist", portal.statement),
                )
            })?;
        let executed = self.run_sql(&prepared.sql, &portal.parameters).await?;
        if let Some(stored) = self.portals.get_mut(name) {
            stored.result = Some(executed.clone());
            stored.cursor = 0;
        }
        Ok(executed)
    }

    /// Column names and types for a statement before it runs: names from the
    /// compiled projection, types from the catalog the graph currently holds.
    async fn describe_statement(
        &self,
        sql: &str,
        placeholders: &[SqlValue],
    ) -> Result<Option<Vec<(String, ColumnType)>>, Failure> {
        let command = compile(sql, placeholders)?;
        Ok(match command {
            SqlCommand::Query(query) if !query.columns.is_empty() => {
                let catalog = self.catalog().await?;
                Some(
                    query
                        .columns
                        .iter()
                        .map(|column| {
                            (
                                column.name.clone(),
                                catalog.column_type(&query, &column.source),
                            )
                        })
                        .collect(),
                )
            }
            SqlCommand::Scalar(scalar) => Some(
                scalar
                    .columns
                    .iter()
                    .zip(&scalar.row)
                    .map(|(column, cell)| {
                        (column.name.clone(), column_type(std::slice::from_ref(cell)))
                    })
                    .collect(),
            ),
            SqlCommand::Catalog(catalog) => Some(
                catalog_columns(&catalog)
                    .into_iter()
                    .map(|(name, kind)| (name.to_owned(), kind))
                    .collect(),
            ),
            SqlCommand::Show(_) => Some(vec![("setting".to_owned(), ColumnType::Text)]),
            _ => None,
        })
    }

    // -- execution ---------------------------------------------------------------

    async fn run_sql(&mut self, sql: &str, parameters: &[SqlValue]) -> Result<Executed, Failure> {
        let command = compile(sql, parameters)?;
        if !matches!(
            self.state.lifecycle.state(),
            LifecycleState::Ready | LifecycleState::Initializing
        ) && !matches!(command, SqlCommand::Empty)
        {
            return Err(Failure::new(
                sqlstate::ADMIN_SHUTDOWN,
                "the server is shutting down",
            ));
        }
        if self.transaction == Transaction::Failed
            && !matches!(
                command,
                SqlCommand::Commit | SqlCommand::Rollback | SqlCommand::Empty
            )
        {
            return Err(Failure::new(
                sqlstate::IN_FAILED_TRANSACTION,
                "current transaction is aborted, commands ignored until end of transaction block",
            ));
        }
        match command {
            SqlCommand::Empty => Ok(Executed::command("")),
            SqlCommand::Set => Ok(Executed::command("SET")),
            SqlCommand::Show(name) => self.show(&name),
            SqlCommand::Begin { read_only } => Ok(self.begin(read_only)),
            SqlCommand::Commit => Ok(self.commit()),
            SqlCommand::Rollback => Ok(self.rollback()),
            SqlCommand::Scalar(scalar) => {
                let columns = scalar
                    .columns
                    .iter()
                    .zip(&scalar.row)
                    .map(|(column, cell)| {
                        (column.name.clone(), column_type(std::slice::from_ref(cell)))
                    })
                    .collect();
                Ok(Executed {
                    columns,
                    rows: vec![scalar.row.iter().map(Cell::to_text).collect()],
                    tag: "SELECT 1".to_owned(),
                    notices: vec![],
                })
            }
            SqlCommand::Catalog(query) => self.catalog_query(&query).await,
            SqlCommand::Query(query) => self.graph_query(sql, query).await,
        }
    }

    fn show(&self, name: &str) -> Result<Executed, Failure> {
        let value = match name {
            "server_version" => SERVER_VERSION.to_owned(),
            "server_version_num" => "160000".to_owned(),
            "client_encoding" | "server_encoding" => "UTF8".to_owned(),
            "standard_conforming_strings" | "integer_datetimes" => "on".to_owned(),
            "transaction_isolation" | "default_transaction_isolation" => {
                "read committed".to_owned()
            }
            "timezone" => "UTC".to_owned(),
            "search_path" => "public".to_owned(),
            "datestyle" => "ISO, MDY".to_owned(),
            "is_superuser" => "off".to_owned(),
            "application_name" => String::new(),
            other => {
                return Err(Failure::new(
                    sqlstate::UNDEFINED_OBJECT,
                    format!("unrecognized configuration parameter \"{other}\""),
                ));
            }
        };
        Ok(Executed {
            columns: vec![(name.to_owned(), ColumnType::Text)],
            rows: vec![vec![Some(value)]],
            tag: "SHOW".to_owned(),
            notices: vec![],
        })
    }

    fn begin(&mut self, read_only: bool) -> Executed {
        match self.transaction {
            Transaction::None => {
                self.transaction = Transaction::Open {
                    read_only,
                    wrote: false,
                };
                Executed::command("BEGIN")
            }
            Transaction::Open { .. } | Transaction::Failed => Executed::command("BEGIN")
                .with_notice(Backend::notice(
                    "WARNING",
                    sqlstate::ACTIVE_TRANSACTION,
                    "there is already a transaction in progress",
                )),
        }
    }

    fn commit(&mut self) -> Executed {
        match self.transaction {
            Transaction::None => Executed::command("COMMIT").with_notice(Backend::notice(
                "WARNING",
                sqlstate::NO_ACTIVE_TRANSACTION,
                "there is no transaction in progress",
            )),
            Transaction::Open { .. } => {
                self.transaction = Transaction::None;
                Executed::command("COMMIT")
            }
            Transaction::Failed => {
                self.transaction = Transaction::None;
                // PostgreSQL answers ROLLBACK to a COMMIT of a failed block.
                Executed::command("ROLLBACK")
            }
        }
    }

    fn rollback(&mut self) -> Executed {
        let previous = self.transaction;
        self.transaction = Transaction::None;
        match previous {
            Transaction::None => Executed::command("ROLLBACK").with_notice(Backend::notice(
                "WARNING",
                sqlstate::NO_ACTIVE_TRANSACTION,
                "there is no transaction in progress",
            )),
            Transaction::Open { wrote: true, .. } => Executed::command("ROLLBACK").with_notice(Backend::notice(
                "WARNING",
                sqlstate::WARNING,
                "statements in this transaction were already applied atomically and durably; ROLLBACK ended the group but did not revert them",
            )),
            Transaction::Open { wrote: false, .. } | Transaction::Failed => Executed::command("ROLLBACK"),
        }
    }

    async fn graph_query(&mut self, sql: &str, query: CompiledQuery) -> Result<Executed, Failure> {
        let read_only_group = matches!(
            self.transaction,
            Transaction::Open {
                read_only: true,
                ..
            }
        );
        if read_only_group && query.mode == QueryMode::Mutation {
            return Err(Failure::new(
                sqlstate::INSUFFICIENT_PRIVILEGE,
                "cannot execute a write statement in a read-only transaction",
            ));
        }
        let mode = match query.mode {
            QueryMode::Read => EngineRequestMode::ReadOnly,
            QueryMode::Mutation => EngineRequestMode::Mutation,
        };
        let request = EngineRequest::new(sql.to_owned(), mode)
            .with_workspace_id(WORKSPACE_ID)
            .with_session_id(self.session_id.clone());
        let ast = query.ast.clone();
        let response = self.execute(request, ast).await?;
        match response.status {
            CypherResponseStatus::Success => {}
            CypherResponseStatus::Rejected | CypherResponseStatus::ValidationFailed => {
                return Err(failure_from_response(&response));
            }
        }
        if query.mode == QueryMode::Mutation
            && let Transaction::Open { wrote, .. } = &mut self.transaction
        {
            *wrote = true;
        }
        let tag_name = match query.tag {
            CommandTag::Select => "SELECT",
            CommandTag::Insert => "INSERT 0",
            CommandTag::Update => "UPDATE",
            CommandTag::Delete => "DELETE",
        };
        match response.data {
            CypherResponseData::Records(records) => {
                let mut rows: Vec<Vec<Cell>> = records
                    .iter()
                    .map(|record| project_row(&query.columns, &record.values))
                    .collect();
                rows = post_process(&query.post, rows);
                let catalog = if rows.iter().any(|row| row.contains(&Cell::Null)) || rows.is_empty()
                {
                    Some(self.catalog().await?)
                } else {
                    None
                };
                let columns = query
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        let cells: Vec<Cell> = rows.iter().map(|row| row[index].clone()).collect();
                        let inferred = column_type(&cells);
                        let all_null = cells.iter().all(|cell| *cell == Cell::Null);
                        let kind = match (&catalog, all_null) {
                            (Some(catalog), true) => catalog.column_type(&query, &column.source),
                            _ => inferred,
                        };
                        (column.name.clone(), kind)
                    })
                    .collect();
                let count = rows.len();
                Ok(Executed {
                    columns,
                    rows: rows
                        .iter()
                        .map(|row| row.iter().map(Cell::to_text).collect())
                        .collect(),
                    tag: format!("{tag_name} {count}"),
                    notices: vec![],
                })
            }
            CypherResponseData::MutationSummary(summary) => {
                let count = match query.tag {
                    CommandTag::Insert => summary.created_nodes + summary.created_relationships,
                    CommandTag::Update => summary.updated_nodes.max(summary.matched_rows),
                    CommandTag::Delete => summary.deleted_nodes + summary.deleted_relationships,
                    CommandTag::Select => 0,
                };
                Ok(Executed::command(&format!("{tag_name} {count}")))
            }
            CypherResponseData::Empty => Ok(Executed::command(&format!("{tag_name} 0"))),
        }
    }

    async fn execute(
        &self,
        request: EngineRequest,
        ast: cypher_parser::QueryAst,
    ) -> Result<CypherResponse, Failure> {
        let engine = Arc::clone(&self.state.engine);
        let timeout = Duration::from_millis(self.state.config.request_timeout_ms);
        let joined = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let mut locked = engine
                    .lock()
                    .map_err(|_| "engine lock poisoned".to_owned())?;
                locked
                    .execute_prepared_request(request, ast)
                    .map_err(|error| error.to_string())
            }),
        )
        .await;
        match joined {
            Err(_) => Err(Failure::new(
                sqlstate::QUERY_CANCELED,
                format!(
                    "the statement exceeded the {} ms request timeout",
                    timeout.as_millis()
                ),
            )),
            Ok(Err(join_error)) => {
                warn!(error = %join_error, "sql execution task failed");
                Err(Failure::new(
                    sqlstate::INTERNAL_ERROR,
                    "the statement could not be executed",
                ))
            }
            Ok(Ok(Err(message))) => {
                warn!(error = %message, "sql execution rejected by the engine");
                Err(Failure::new(
                    sqlstate::INTERNAL_ERROR,
                    "the statement could not be executed",
                ))
            }
            Ok(Ok(Ok(response))) => Ok(response),
        }
    }

    // -- catalog ---------------------------------------------------------------------

    /// A snapshot of labels and property types, built from the graph the
    /// engine holds. Paged deployments see what is currently projected.
    async fn catalog(&self) -> Result<Catalog, Failure> {
        let engine = Arc::clone(&self.state.engine);
        tokio::task::spawn_blocking(move || {
            let mut locked = engine
                .lock()
                .map_err(|_| "engine lock poisoned".to_owned())?;
            let _ = locked.hydrate_full_graph();
            let graph = locked.graph();
            let nodes = graph.list_nodes().map_err(|error| error.to_string())?;
            let relationships = graph
                .list_relationships()
                .map_err(|error| error.to_string())?;
            let mut catalog = Catalog::default();
            for node in &nodes {
                for label in node.labels() {
                    let table = catalog.tables.entry(label.clone()).or_default();
                    for (key, value) in node.properties() {
                        table.observe(key, value);
                    }
                }
            }
            for relationship in &relationships {
                let table = catalog
                    .relationship_properties
                    .entry(relationship.rel_type().as_str().to_owned())
                    .or_default();
                for (key, value) in relationship.properties() {
                    table.observe(key, value);
                }
            }
            Ok::<Catalog, String>(catalog)
        })
        .await
        .map_err(|error| {
            warn!(error = %error, "catalog task failed");
            Failure::new(sqlstate::INTERNAL_ERROR, "the catalog could not be read")
        })?
        .map_err(|message| {
            warn!(error = %message, "catalog read failed");
            Failure::new(sqlstate::INTERNAL_ERROR, "the catalog could not be read")
        })
    }

    async fn catalog_query(&self, query: &CatalogQuery) -> Result<Executed, Failure> {
        let catalog = self.catalog().await?;
        let all_columns = catalog_columns(query);
        let selected: Vec<(String, ColumnType)> = if query.columns.is_empty() {
            all_columns
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), *kind))
                .collect()
        } else {
            let mut selected = Vec::new();
            for requested in &query.columns {
                let Some((name, kind)) = all_columns
                    .iter()
                    .find(|(name, _)| *name == requested.as_str())
                else {
                    return Err(Failure::new(
                        sqlstate::UNDEFINED_OBJECT,
                        format!(
                            "column \"{requested}\" does not exist in this information_schema view"
                        ),
                    ));
                };
                selected.push(((*name).to_owned(), *kind));
            }
            selected
        };
        let mut rows: Vec<BTreeMap<&str, Cell>> = match query.kind {
            CatalogKind::Tables => catalog
                .table_names()
                .into_iter()
                .filter(|name| query.table.as_deref().is_none_or(|wanted| wanted == name))
                .map(|name| {
                    BTreeMap::from([
                        ("table_catalog", Cell::Text("corrobore".to_owned())),
                        ("table_schema", Cell::Text("public".to_owned())),
                        ("table_name", Cell::Text(name)),
                        ("table_type", Cell::Text("BASE TABLE".to_owned())),
                    ])
                })
                .collect(),
            CatalogKind::Columns => catalog
                .table_names()
                .into_iter()
                .filter(|name| query.table.as_deref().is_none_or(|wanted| wanted == name))
                .flat_map(|table| {
                    catalog
                        .columns_of(&table)
                        .into_iter()
                        .enumerate()
                        .map(|(position, (column, kind))| {
                            BTreeMap::from([
                                ("table_catalog", Cell::Text("corrobore".to_owned())),
                                ("table_schema", Cell::Text("public".to_owned())),
                                ("table_name", Cell::Text(table.clone())),
                                ("column_name", Cell::Text(column)),
                                ("ordinal_position", Cell::Int(position as i64 + 1)),
                                ("data_type", Cell::Text(kind.data_type_name().to_owned())),
                                ("is_nullable", Cell::Text("YES".to_owned())),
                            ])
                        })
                        .collect::<Vec<_>>()
                })
                .collect(),
        };
        if let Some(order) = &query.order_by {
            rows.sort_by(|left, right| {
                format!("{:?}", left.get(order.as_str()))
                    .cmp(&format!("{:?}", right.get(order.as_str())))
            });
        }
        let data: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| {
                selected
                    .iter()
                    .map(|(name, _)| row.get(name.as_str()).and_then(Cell::to_text))
                    .collect()
            })
            .collect();
        Ok(Executed {
            columns: selected,
            tag: format!("SELECT {}", data.len()),
            rows: data,
            notices: vec![],
        })
    }
}

/// Column names and types of an `information_schema` view.
fn catalog_columns(query: &CatalogQuery) -> Vec<(&'static str, ColumnType)> {
    match query.kind {
        CatalogKind::Tables => vec![
            ("table_catalog", ColumnType::Text),
            ("table_schema", ColumnType::Text),
            ("table_name", ColumnType::Text),
            ("table_type", ColumnType::Text),
        ],
        CatalogKind::Columns => vec![
            ("table_catalog", ColumnType::Text),
            ("table_schema", ColumnType::Text),
            ("table_name", ColumnType::Text),
            ("column_name", ColumnType::Text),
            ("ordinal_position", ColumnType::Int8),
            ("data_type", ColumnType::Text),
            ("is_nullable", ColumnType::Text),
        ],
    }
}

/// Property types observed for one table.
#[derive(Debug, Default)]
struct TableTypes {
    properties: BTreeMap<String, Option<ColumnType>>,
}

impl TableTypes {
    fn observe(&mut self, key: &str, value: &PropertyValue) {
        let observed = match value {
            PropertyValue::Null => return,
            PropertyValue::Bool(_) => ColumnType::Bool,
            PropertyValue::Integer(_) => ColumnType::Int8,
            PropertyValue::Float(_) => ColumnType::Float8,
            PropertyValue::String(_) => ColumnType::Text,
            PropertyValue::StringList(_)
            | PropertyValue::IntegerList(_)
            | PropertyValue::FloatList(_)
            | PropertyValue::BoolList(_)
            | PropertyValue::Json(_) => ColumnType::Jsonb,
        };
        let entry = self
            .properties
            .entry(key.to_owned())
            .or_insert(Some(observed));
        *entry = match *entry {
            None => Some(observed),
            Some(known) if known == observed => Some(known),
            Some(ColumnType::Int8) if observed == ColumnType::Float8 => Some(ColumnType::Float8),
            Some(ColumnType::Float8) if observed == ColumnType::Int8 => Some(ColumnType::Float8),
            Some(_) => Some(ColumnType::Text),
        };
    }

    fn get(&self, key: &str) -> Option<ColumnType> {
        self.properties.get(key).copied().flatten()
    }
}

#[derive(Debug, Default)]
struct Catalog {
    tables: BTreeMap<String, TableTypes>,
    relationship_properties: BTreeMap<String, TableTypes>,
}

const NODE_STRUCTURAL: [(&str, ColumnType); 6] = [
    ("id", ColumnType::Text),
    ("labels", ColumnType::Jsonb),
    ("status", ColumnType::Text),
    ("confidence", ColumnType::Float8),
    ("evidence_refs", ColumnType::Jsonb),
    ("properties", ColumnType::Jsonb),
];

const RELATIONSHIP_STRUCTURAL: [(&str, ColumnType); 8] = [
    ("id", ColumnType::Text),
    ("type", ColumnType::Text),
    ("source_id", ColumnType::Text),
    ("target_id", ColumnType::Text),
    ("status", ColumnType::Text),
    ("confidence", ColumnType::Float8),
    ("evidence_refs", ColumnType::Jsonb),
    ("properties", ColumnType::Jsonb),
];

impl Catalog {
    fn table_names(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self.tables.keys().cloned().collect();
        names.insert("nodes".to_owned());
        names.insert("relationships".to_owned());
        names.into_iter().collect()
    }

    fn columns_of(&self, table: &str) -> Vec<(String, ColumnType)> {
        let mut columns: Vec<(String, ColumnType)> = if table == "relationships" {
            RELATIONSHIP_STRUCTURAL
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), *kind))
                .collect()
        } else {
            NODE_STRUCTURAL
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), *kind))
                .collect()
        };
        if let Some(types) = self.tables.get(table) {
            for (key, kind) in &types.properties {
                if !columns.iter().any(|(name, _)| name == key) {
                    columns.push((key.clone(), kind.unwrap_or(ColumnType::Text)));
                }
            }
        }
        columns
    }

    /// The type a column would have, from the catalog, for a description that
    /// must be sent before rows exist or when every row is NULL.
    fn column_type(&self, query: &CompiledQuery, source: &ColumnSource) -> ColumnType {
        let label_of = |variable: &str| -> Option<String> {
            let parsed = query.ast.query.as_ref()?;
            let matched = parsed.match_clause.as_ref()?;
            if matched.start.variable == variable {
                return matched.start.label.clone();
            }
            if let Some((relationship, target)) = &matched.relationship {
                if target.variable == variable {
                    return target.label.clone();
                }
                if relationship.variable.as_deref() == Some(variable) {
                    return relationship
                        .rel_type
                        .clone()
                        .map(|kind| format!("\u{1}{kind}"));
                }
            }
            parsed
                .create_clause
                .as_ref()
                .and_then(|create| create.nodes.first())
                .filter(|node| node.variable == variable)
                .and_then(|node| node.label.clone())
        };
        let property_type = |variable: &str, property: &str| -> ColumnType {
            match label_of(variable) {
                Some(label) if label.starts_with('\u{1}') => self
                    .relationship_properties
                    .get(&label[1..])
                    .and_then(|types| types.get(property)),
                Some(label) => self
                    .tables
                    .get(&label)
                    .and_then(|types| types.get(property)),
                None => self.tables.values().find_map(|types| types.get(property)),
            }
            .unwrap_or(ColumnType::Text)
        };
        match source {
            ColumnSource::Constant(cell) => column_type(std::slice::from_ref(cell)),
            ColumnSource::Field(key) => {
                if key == "count" {
                    return ColumnType::Int8;
                }
                if let Some(inner) = key.strip_prefix("avg(") {
                    let _ = inner;
                    return ColumnType::Float8;
                }
                for aggregate in ["sum(", "min(", "max("] {
                    if let Some(inner) = key.strip_prefix(aggregate) {
                        let inner = inner.trim_end_matches(')');
                        return inner
                            .split_once('.')
                            .map_or(ColumnType::Text, |(variable, property)| {
                                property_type(variable, property)
                            });
                    }
                }
                key.split_once('.').map_or(
                    ColumnType::Text,
                    |(variable, property)| match property {
                        "id" | "status" => ColumnType::Text,
                        "confidence" => ColumnType::Float8,
                        _ => property_type(variable, property),
                    },
                )
            }
            ColumnSource::Entity { variable, field } => match field {
                EntityField::Id
                | EntityField::Type
                | EntityField::SourceId
                | EntityField::TargetId
                | EntityField::Status => ColumnType::Text,
                EntityField::Confidence => ColumnType::Float8,
                EntityField::Labels | EntityField::EvidenceRefs | EntityField::Properties => {
                    ColumnType::Jsonb
                }
                EntityField::Property(property) => property_type(variable, property),
            },
        }
    }
}

/// Translate the runtime's rejection vocabulary into SQLSTATEs.
fn failure_from_response(response: &CypherResponse) -> Failure {
    let first = response.validation_errors.first();
    let message = first
        .map(|error| error.message.clone())
        .or_else(|| response.warnings.first().cloned())
        .unwrap_or_else(|| "the statement was rejected".to_owned());
    let sqlstate = match first.map(|error| error.code.as_str()) {
        Some("WRITE_PERMISSION_REQUIRED" | "REQUEST_MODE_DISALLOWED") => {
            sqlstate::INSUFFICIENT_PRIVILEGE
        }
        Some("QUERY_BUDGET_EXCEEDED" | "REQUEST_LIMIT_EXCEEDED") => {
            sqlstate::PROGRAM_LIMIT_EXCEEDED
        }
        _ => sqlstate::SYNTAX_ERROR,
    };
    debug!(sqlstate, %message, "sql statement rejected by the runtime");
    Failure::new(sqlstate, message)
}

/// Decode one bound parameter from its wire representation.
fn decode_parameter(bytes: &[u8], format: i16, oid: i32) -> Result<SqlValue, Failure> {
    let malformed = |what: &str| {
        Failure::new(
            sqlstate::PROTOCOL_VIOLATION,
            format!("malformed {what} parameter"),
        )
    };
    if format == 1 {
        return Ok(match oid {
            OID_BOOL => SqlValue::Boolean(bytes.first().copied().unwrap_or(0) != 0),
            OID_INT2 => SqlValue::Integer(i64::from(i16::from_be_bytes(
                bytes.try_into().map_err(|_| malformed("int2"))?,
            ))),
            OID_INT4 => SqlValue::Integer(i64::from(i32::from_be_bytes(
                bytes.try_into().map_err(|_| malformed("int4"))?,
            ))),
            OID_INT8 => SqlValue::Integer(i64::from_be_bytes(
                bytes.try_into().map_err(|_| malformed("int8"))?,
            )),
            OID_FLOAT4 => SqlValue::Float(
                f32::from_be_bytes(bytes.try_into().map_err(|_| malformed("float4"))?).to_string(),
            ),
            OID_FLOAT8 => SqlValue::Float(
                f64::from_be_bytes(bytes.try_into().map_err(|_| malformed("float8"))?).to_string(),
            ),
            OID_TEXT | OID_VARCHAR | 0 => {
                SqlValue::Text(String::from_utf8(bytes.to_vec()).map_err(|_| malformed("text"))?)
            }
            other => {
                return Err(Failure::new(
                    sqlstate::PROTOCOL_VIOLATION,
                    format!("binary parameters of type oid {other} are not supported"),
                ));
            }
        });
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| malformed("text"))?;
    Ok(match oid {
        OID_BOOL => match text.as_str() {
            "t" | "true" | "TRUE" | "1" | "y" | "yes" | "on" => SqlValue::Boolean(true),
            "f" | "false" | "FALSE" | "0" | "n" | "no" | "off" => SqlValue::Boolean(false),
            _ => return Err(malformed("boolean")),
        },
        OID_INT2 | OID_INT4 | OID_INT8 => {
            SqlValue::Integer(text.trim().parse().map_err(|_| malformed("integer"))?)
        }
        OID_FLOAT4 | OID_FLOAT8 | OID_NUMERIC => {
            text.trim().parse::<f64>().map_err(|_| malformed("float"))?;
            SqlValue::Float(text.trim().to_owned())
        }
        OID_TEXT | OID_VARCHAR => SqlValue::Text(text),
        // An unspecified type is read the way a literal would be: a driver that
        // sends every value as text still gets integers and booleans bound as
        // such, which is what a typed graph comparison needs.
        _ => infer_text_parameter(&text),
    })
}

fn infer_text_parameter(text: &str) -> SqlValue {
    let trimmed = text.trim();
    if let Ok(integer) = trimmed.parse::<i64>() {
        return SqlValue::Integer(integer);
    }
    if trimmed.parse::<f64>().is_ok()
        && trimmed.chars().any(|c| c.is_ascii_digit())
        && !trimmed.eq_ignore_ascii_case("nan")
        && !trimmed.to_ascii_lowercase().contains("inf")
    {
        return SqlValue::Float(trimmed.to_owned());
    }
    match trimmed {
        "true" | "TRUE" | "t" => SqlValue::Boolean(true),
        "false" | "FALSE" | "f" => SqlValue::Boolean(false),
        "null" | "NULL" => SqlValue::Null,
        _ => SqlValue::Text(text.to_owned()),
    }
}
