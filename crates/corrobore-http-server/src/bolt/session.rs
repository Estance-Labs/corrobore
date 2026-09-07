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
//! One Bolt connection's state machine.
//!
//! Module boundary: this module decides what a message means in the current
//! state and which responses it produces. It reads no socket and frames no
//! bytes; the listener does that. Every query still enters the engine through
//! `CorroboreEngine::execute_request`, so policy, budgets, persistence and the
//! audit path are the ones every other adapter uses.
//!
//! Transactions are statement groups. Each statement is applied through the
//! engine's per-request atomic mutation model and is durable on its own, which
//! is why a `ROLLBACK` after an applied write is refused with a stable code
//! instead of pretending. A read-only group rolls back trivially.

use std::{
    collections::{BTreeMap, VecDeque},
    hash::Hasher,
    sync::Arc,
    time::{Duration, Instant},
};

use corrobore_engine::{EngineRequest, EngineRequestMode};
use shared_runtime::{
    CypherResponse, CypherResponseData, CypherResponseStatus, CypherValue, RecordValue,
    contains_mutation_keywords,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::packstream::Value;
use crate::{
    app::AppState, auth::token_matches, lifecycle::LifecycleState, security::AuthenticationMode,
};

// Request signatures.
pub(super) const HELLO: u8 = 0x01;
pub(super) const GOODBYE: u8 = 0x02;
pub(super) const RESET: u8 = 0x0F;
pub(super) const RUN: u8 = 0x10;
pub(super) const BEGIN: u8 = 0x11;
pub(super) const COMMIT: u8 = 0x12;
pub(super) const ROLLBACK: u8 = 0x13;
pub(super) const DISCARD: u8 = 0x2F;
pub(super) const PULL: u8 = 0x3F;
pub(super) const TELEMETRY: u8 = 0x54;
pub(super) const ROUTE: u8 = 0x66;
pub(super) const LOGON: u8 = 0x6A;
pub(super) const LOGOFF: u8 = 0x6B;

// Response signatures.
pub(super) const SUCCESS: u8 = 0x70;
pub(super) const RECORD: u8 = 0x71;
pub(super) const IGNORED: u8 = 0x7E;
pub(super) const FAILURE: u8 = 0x7F;

/// The one database this server exposes.
const DATABASE: &str = "corrobore";
const WORKSPACE_ID: &str = "workspace--bolt-default";
/// Routing table lifetime advertised to `neo4j://` clients.
const ROUTING_TTL_SECONDS: i64 = 300;

/// Stable failure codes. The class prefix is what drivers act on:
/// `ClientError` is final, `TransientError` may be retried, `DatabaseError`
/// is a server fault.
pub(super) mod code {
    pub const UNAUTHORIZED: &str = "Neo.ClientError.Security.Unauthorized";
    pub const FORBIDDEN: &str = "Neo.ClientError.Security.Forbidden";
    pub const SYNTAX: &str = "Neo.ClientError.Statement.SyntaxError";
    pub const ARGUMENT: &str = "Neo.ClientError.Statement.ArgumentError";
    pub const REQUEST_INVALID: &str = "Neo.ClientError.Request.Invalid";
    pub const ROLLBACK_UNSUPPORTED: &str = "Neo.ClientError.Transaction.RollbackNotSupported";
    pub const TIMED_OUT: &str = "Neo.ClientError.Transaction.TransactionTimedOut";
    pub const BUDGET: &str = "Neo.TransientError.General.ResourceBudgetExceeded";
    pub const UNAVAILABLE: &str = "Neo.TransientError.General.DatabaseUnavailable";
    pub const EXECUTION_FAILED: &str = "Neo.DatabaseError.Statement.ExecutionFailed";
}

/// A response message to write.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Response {
    pub tag: u8,
    pub fields: Vec<Value>,
}

impl Response {
    fn success(metadata: Vec<(String, Value)>) -> Self {
        Self {
            tag: SUCCESS,
            fields: vec![Value::Dictionary(metadata)],
        }
    }

    fn record(values: Vec<Value>) -> Self {
        Self {
            tag: RECORD,
            fields: vec![Value::List(values)],
        }
    }

    fn ignored() -> Self {
        Self {
            tag: IGNORED,
            fields: vec![],
        }
    }

    /// Bolt 5.7 replaced `code` with `neo4j_code` beside a GQL status; both
    /// vocabularies are sent so every driver version reads the same failure.
    fn failure_for(code: &str, message: impl Into<String>, gql: bool) -> Self {
        let message = message.into();
        let mut metadata = vec![
            ("code".to_owned(), Value::String(code.to_owned())),
            ("message".to_owned(), Value::String(message.clone())),
        ];
        if gql {
            let gql_status = if code.starts_with("Neo.ClientError.Statement.") {
                "42000"
            } else if code.starts_with("Neo.ClientError.Security.") {
                "42N42"
            } else {
                "50N42"
            };
            metadata.push(("neo4j_code".to_owned(), Value::String(code.to_owned())));
            metadata.push((
                "gql_status".to_owned(),
                Value::String(gql_status.to_owned()),
            ));
            metadata.push((
                "status_description".to_owned(),
                Value::String(format!("error: {message}")),
            ));
            metadata.push(("description".to_owned(), Value::String(message)));
            metadata.push((
                "diagnostic_record".to_owned(),
                Value::Dictionary(vec![
                    ("OPERATION".to_owned(), Value::String(String::new())),
                    ("OPERATION_CODE".to_owned(), Value::String("0".to_owned())),
                    ("CURRENT_SCHEMA".to_owned(), Value::String("/".to_owned())),
                ]),
            ));
        }
        Self {
            tag: FAILURE,
            fields: vec![Value::Dictionary(metadata)],
        }
    }
}

/// What the listener does after a message: write these responses, and either
/// keep the connection or close it.
#[derive(Debug, PartialEq)]
pub(super) struct Outcome {
    pub responses: Vec<Response>,
    pub close: bool,
}

impl Outcome {
    fn reply(response: Response) -> Self {
        Self {
            responses: vec![response],
            close: false,
        }
    }

    fn close_after(response: Response) -> Self {
        Self {
            responses: vec![response],
            close: true,
        }
    }

    fn close_silently() -> Self {
        Self {
            responses: vec![],
            close: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Version negotiated, no HELLO yet.
    Negotiated,
    /// HELLO accepted, credentials still owed (Bolt 5.1+).
    Authentication,
    /// Authenticated, no open stream or transaction.
    Ready,
    /// Autocommit result waiting to be pulled.
    Streaming,
    /// Explicit transaction open, no result pending.
    TxReady,
    /// Explicit transaction open with results pending.
    TxStreaming,
    /// A FAILURE was sent; everything but RESET is IGNORED.
    Failed,
}

/// A buffered result waiting for PULL or DISCARD.
struct Stream {
    records: VecDeque<Vec<Value>>,
    summary: Vec<(String, Value)>,
}

struct Transaction {
    read_only: bool,
    /// Whether a statement in this group applied a mutation.
    wrote: bool,
}

pub(super) struct Session {
    state: Arc<AppState>,
    major: u8,
    minor: u8,
    phase: State,
    connection_id: String,
    session_id: String,
    streams: BTreeMap<i64, Stream>,
    next_qid: i64,
    transaction: Option<Transaction>,
    /// Restored by RESET: whether HELLO carried credentials already.
    authenticated: bool,
}

impl Session {
    pub(super) fn new(state: Arc<AppState>, major: u8, minor: u8) -> Self {
        let id = Uuid::new_v4();
        Self {
            state,
            major,
            minor,
            phase: State::Negotiated,
            connection_id: format!("bolt-{id}"),
            session_id: format!("session--bolt-{id}"),
            streams: BTreeMap::new(),
            next_qid: 0,
            transaction: None,
            authenticated: false,
        }
    }

    fn at_least(&self, major: u8, minor: u8) -> bool {
        (self.major, self.minor) >= (major, minor)
    }

    /// Bolt 5.1 moved credentials from HELLO to LOGON.
    fn logon_required(&self) -> bool {
        self.at_least(5, 1)
    }

    fn failure(&self, code: &str, message: impl Into<String>) -> Response {
        Response::failure_for(code, message, self.at_least(5, 7))
    }

    fn fail(&mut self, code: &str, message: impl Into<String>) -> Outcome {
        self.phase = State::Failed;
        Outcome::reply(self.failure(code, message))
    }

    fn protocol_violation(&mut self, message: &str) -> Outcome {
        // The specification closes the connection on a protocol violation, so
        // a confused client does not keep a slot busy.
        Outcome::close_after(self.failure(code::REQUEST_INVALID, message))
    }

    /// Route one decoded message.
    pub(super) async fn handle(&mut self, message: Value) -> Outcome {
        let Value::Structure { tag, fields } = message else {
            return self.protocol_violation("a Bolt message is a PackStream structure");
        };
        if tag == GOODBYE {
            return Outcome::close_silently();
        }
        if tag == RESET {
            return self.reset();
        }
        match self.phase {
            State::Failed => Outcome::reply(Response::ignored()),
            State::Negotiated => {
                if tag == HELLO {
                    self.hello(fields)
                } else {
                    self.protocol_violation("HELLO must be the first message")
                }
            }
            State::Authentication => match tag {
                LOGON => self.logon(fields),
                ROUTE => self.route(fields),
                _ => self.fail(
                    code::UNAUTHORIZED,
                    "authentication is required before this request",
                ),
            },
            State::Ready | State::Streaming | State::TxReady | State::TxStreaming => {
                self.ready_message(tag, fields).await
            }
        }
    }

    fn reset(&mut self) -> Outcome {
        self.streams.clear();
        self.transaction = None;
        self.phase = if self.authenticated {
            State::Ready
        } else if self.logon_required() {
            State::Authentication
        } else {
            State::Negotiated
        };
        Outcome::reply(Response::success(vec![]))
    }

    fn hello(&mut self, fields: Vec<Value>) -> Outcome {
        let Some(extra) = fields.first() else {
            return self.protocol_violation("HELLO carries a metadata dictionary");
        };
        let user_agent = extra
            .get("user_agent")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        info!(
            connection_id = %self.connection_id,
            bolt_version = %format!("{}.{}", self.major, self.minor),
            user_agent = %user_agent,
            "bolt hello"
        );
        if !self.logon_required() {
            // Bolt 4.4 and 5.0 authenticate inline.
            if let Err(outcome) = self.authenticate(extra) {
                return outcome;
            }
            self.authenticated = true;
            self.phase = State::Ready;
        } else {
            self.phase = State::Authentication;
        }
        Outcome::reply(Response::success(vec![
            (
                "server".to_owned(),
                Value::String(format!("Corrobore/{}", env!("CARGO_PKG_VERSION"))),
            ),
            (
                "connection_id".to_owned(),
                Value::String(self.connection_id.clone()),
            ),
        ]))
    }

    fn logon(&mut self, fields: Vec<Value>) -> Outcome {
        let Some(auth) = fields.first() else {
            return self.protocol_violation("LOGON carries an authentication dictionary");
        };
        if let Err(outcome) = self.authenticate(auth) {
            return outcome;
        }
        self.authenticated = true;
        self.phase = State::Ready;
        Outcome::reply(Response::success(vec![]))
    }

    /// Check credentials against the same bearer token HTTP requires. A failed
    /// login closes the connection: a driver retries with a new one, and a
    /// guesser does not get a second attempt on the same slot.
    fn authenticate(&mut self, auth: &Value) -> Result<(), Outcome> {
        let scheme = auth.get("scheme").and_then(Value::as_str).unwrap_or("none");
        let credentials = auth.get("credentials").and_then(Value::as_str);
        let config = &self.state.config;
        let accepted = match config.auth_mode {
            AuthenticationMode::LocalInsecure => true,
            AuthenticationMode::Required => {
                match (scheme, credentials, config.auth_token.as_deref()) {
                    ("basic" | "bearer", Some(provided), Some(expected)) => {
                        token_matches(provided, expected)
                    }
                    _ => false,
                }
            }
        };
        if accepted {
            debug!(connection_id = %self.connection_id, scheme, "bolt authenticated");
            Ok(())
        } else {
            warn!(connection_id = %self.connection_id, scheme, "bolt authentication refused");
            Err(Outcome::close_after(self.failure(
                code::UNAUTHORIZED,
                "the supplied credentials were not accepted; use the Corrobore bearer token as the password of a basic scheme or as a bearer credential",
            )))
        }
    }

    async fn ready_message(&mut self, tag: u8, fields: Vec<Value>) -> Outcome {
        match tag {
            HELLO => self.protocol_violation("HELLO was already sent on this connection"),
            LOGON => self.fail(
                code::REQUEST_INVALID,
                "the connection is already authenticated",
            ),
            LOGOFF if self.logon_required() => {
                self.authenticated = false;
                self.streams.clear();
                self.transaction = None;
                self.phase = State::Authentication;
                Outcome::reply(Response::success(vec![]))
            }
            RUN => self.run(fields).await,
            PULL => self.pull(fields, false),
            DISCARD => self.pull(fields, true),
            BEGIN => self.begin(fields),
            COMMIT => self.commit(),
            ROLLBACK => self.rollback(),
            ROUTE => self.route(fields),
            TELEMETRY if self.at_least(5, 4) => Outcome::reply(Response::success(vec![])),
            other => self.protocol_violation(&format!("unknown message signature {other:#04x}")),
        }
    }

    fn begin(&mut self, fields: Vec<Value>) -> Outcome {
        if self.phase != State::Ready {
            return self.fail(
                code::REQUEST_INVALID,
                "BEGIN requires a ready session with no open transaction or stream",
            );
        }
        let read_only = fields
            .first()
            .and_then(|extra| extra.get("mode"))
            .and_then(Value::as_str)
            == Some("r");
        self.transaction = Some(Transaction {
            read_only,
            wrote: false,
        });
        self.phase = State::TxReady;
        Outcome::reply(Response::success(vec![]))
    }

    fn commit(&mut self) -> Outcome {
        if !matches!(self.phase, State::TxReady | State::TxStreaming) {
            return self.fail(code::REQUEST_INVALID, "COMMIT requires an open transaction");
        }
        // Pending streams die with the transaction; every statement is already
        // applied, so nothing is lost but unread rows.
        self.streams.clear();
        self.transaction = None;
        self.phase = State::Ready;
        Outcome::reply(Response::success(vec![(
            "bookmark".to_owned(),
            Value::String(bookmark()),
        )]))
    }

    fn rollback(&mut self) -> Outcome {
        if !matches!(self.phase, State::TxReady | State::TxStreaming) {
            return self.fail(
                code::REQUEST_INVALID,
                "ROLLBACK requires an open transaction",
            );
        }
        let wrote = self.transaction.as_ref().is_some_and(|tx| tx.wrote);
        self.streams.clear();
        self.transaction = None;
        if wrote {
            return self.fail(
                code::ROLLBACK_UNSUPPORTED,
                "each statement in a Corrobore transaction is applied atomically and durably on its own; a write already applied cannot be rolled back",
            );
        }
        self.phase = State::Ready;
        Outcome::reply(Response::success(vec![]))
    }

    fn route(&mut self, fields: Vec<Value>) -> Outcome {
        if !self.at_least(4, 3) {
            return self.protocol_violation("ROUTE requires Bolt 4.3 or later");
        }
        let address = fields
            .first()
            .and_then(|routing| routing.get("address"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!("{}:{}", self.state.config.host, self.state.config.bolt_port)
            });
        let server = |role: &str| {
            Value::Dictionary(vec![
                (
                    "addresses".to_owned(),
                    Value::List(vec![Value::String(address.clone())]),
                ),
                ("role".to_owned(), Value::String(role.to_owned())),
            ])
        };
        Outcome::reply(Response::success(vec![(
            "rt".to_owned(),
            Value::Dictionary(vec![
                ("ttl".to_owned(), Value::Integer(ROUTING_TTL_SECONDS)),
                ("db".to_owned(), Value::String(DATABASE.to_owned())),
                (
                    "servers".to_owned(),
                    Value::List(vec![server("WRITE"), server("READ"), server("ROUTE")]),
                ),
            ]),
        )]))
    }

    async fn run(&mut self, fields: Vec<Value>) -> Outcome {
        if self.state.lifecycle.state() != LifecycleState::Ready
            && self.state.lifecycle.state() != LifecycleState::Initializing
        {
            return self.fail(code::UNAVAILABLE, "the server is shutting down");
        }
        let in_transaction = matches!(self.phase, State::TxReady | State::TxStreaming);
        if self.phase == State::Streaming {
            return self.fail(
                code::REQUEST_INVALID,
                "an autocommit result is still open; PULL or DISCARD it before the next RUN",
            );
        }
        let mut fields = fields.into_iter();
        let Some(Value::String(query)) = fields.next() else {
            return self.protocol_violation("RUN carries the query text first");
        };
        let parameters = match fields.next() {
            Some(Value::Dictionary(entries)) => entries,
            None => vec![],
            Some(_) => return self.protocol_violation("RUN parameters are a dictionary"),
        };
        let extra = fields.next().unwrap_or(Value::Dictionary(vec![]));

        let mut typed = std::collections::HashMap::new();
        for (name, value) in parameters {
            match to_cypher_value(&value) {
                Some(value) => {
                    typed.insert(name, value);
                }
                None => {
                    return self.fail(
                        code::ARGUMENT,
                        format!(
                            "parameter `{name}` has a type Corrobore Cypher cannot bind; use null, boolean, integer, float, string or a list of those"
                        ),
                    );
                }
            }
        }

        let read_only = match &self.transaction {
            Some(transaction) => transaction.read_only,
            None => extra.get("mode").and_then(Value::as_str) == Some("r"),
        };
        let mode = if read_only {
            EngineRequestMode::ReadOnly
        } else {
            EngineRequestMode::Auto
        };
        let writes = contains_mutation_keywords(&query);
        let request = EngineRequest::new(query.clone(), mode)
            .with_typed_parameters(typed)
            .with_workspace_id(WORKSPACE_ID)
            .with_session_id(self.session_id.clone());

        let started = Instant::now();
        let response = match self.execute(request).await {
            Ok(response) => response,
            Err((code, message)) => return self.fail(code, message),
        };
        let t_first = elapsed_ms(started);

        match response.status {
            CypherResponseStatus::Success => {}
            CypherResponseStatus::Rejected | CypherResponseStatus::ValidationFailed => {
                let (code, message) = failure_from_response(&response);
                return self.fail(code, message);
            }
        }

        let (columns, records, mut summary, wrote) = match response.data {
            CypherResponseData::Records(records) => {
                let columns = response.columns;
                let rows = records
                    .into_iter()
                    .map(|record| {
                        columns
                            .iter()
                            .map(|column| {
                                record
                                    .values
                                    .get(column)
                                    .map(|value| self.encode_value(value))
                                    .unwrap_or(Value::Null)
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<VecDeque<_>>();
                let summary = vec![(
                    "type".to_owned(),
                    Value::String(if writes { "rw" } else { "r" }.to_owned()),
                )];
                (columns, rows, summary, writes)
            }
            CypherResponseData::MutationSummary(counters) => {
                let stats = Value::Dictionary(vec![
                    (
                        "nodes-created".to_owned(),
                        Value::Integer(counters.created_nodes as i64),
                    ),
                    (
                        "nodes-deleted".to_owned(),
                        Value::Integer(counters.deleted_nodes as i64),
                    ),
                    (
                        "relationships-created".to_owned(),
                        Value::Integer(counters.created_relationships as i64),
                    ),
                    (
                        "relationships-deleted".to_owned(),
                        Value::Integer(counters.deleted_relationships as i64),
                    ),
                    (
                        "properties-set".to_owned(),
                        Value::Integer(counters.properties_set as i64),
                    ),
                ]);
                let summary = vec![
                    ("type".to_owned(), Value::String("w".to_owned())),
                    ("stats".to_owned(), stats),
                ];
                (vec![], VecDeque::new(), summary, true)
            }
            CypherResponseData::Empty => (
                vec![],
                VecDeque::new(),
                vec![("type".to_owned(), Value::String("r".to_owned()))],
                false,
            ),
        };
        summary.push(("db".to_owned(), Value::String(DATABASE.to_owned())));
        if in_transaction {
            if let Some(transaction) = self.transaction.as_mut() {
                transaction.wrote |= wrote;
            }
        } else if wrote {
            summary.push(("bookmark".to_owned(), Value::String(bookmark())));
        }

        let qid = self.next_qid;
        self.next_qid += 1;
        self.streams.insert(qid, Stream { records, summary });
        self.phase = if in_transaction {
            State::TxStreaming
        } else {
            State::Streaming
        };

        let mut metadata = vec![
            (
                "fields".to_owned(),
                Value::List(columns.into_iter().map(Value::String).collect()),
            ),
            ("t_first".to_owned(), Value::Integer(t_first)),
        ];
        if in_transaction {
            metadata.push(("qid".to_owned(), Value::Integer(qid)));
        }
        Outcome::reply(Response::success(metadata))
    }

    /// PULL streams up to `n` records; DISCARD drops them. Both end the stream
    /// with its summary once it is exhausted, or report `has_more`.
    fn pull(&mut self, fields: Vec<Value>, discard: bool) -> Outcome {
        if !matches!(self.phase, State::Streaming | State::TxStreaming) {
            return self.fail(
                code::REQUEST_INVALID,
                "there is no open result to pull from",
            );
        }
        let extra = fields.first().cloned().unwrap_or(Value::Dictionary(vec![]));
        let n = extra.get("n").and_then(Value::as_i64).unwrap_or(-1);
        let requested = extra.get("qid").and_then(Value::as_i64).unwrap_or(-1);
        let qid = if requested < 0 {
            self.streams.keys().next_back().copied()
        } else if self.streams.contains_key(&requested) {
            Some(requested)
        } else {
            None
        };
        let Some(qid) = qid else {
            return self.fail(code::REQUEST_INVALID, "the requested result does not exist");
        };
        if n == 0 || n < -1 {
            return self.fail(
                code::REQUEST_INVALID,
                "`n` must be -1 or a positive integer",
            );
        }

        let mut responses = Vec::new();
        let stream = self.streams.get_mut(&qid).expect("qid was just checked");
        if discard {
            stream.records.clear();
        } else {
            let take = if n < 0 { usize::MAX } else { n as usize };
            let mut sent = 0usize;
            while sent < take {
                match stream.records.pop_front() {
                    Some(values) => {
                        responses.push(Response::record(values));
                        sent += 1;
                    }
                    None => break,
                }
            }
        }
        if stream.records.is_empty() {
            let stream = self.streams.remove(&qid).expect("stream exists");
            let mut summary = stream.summary;
            summary.push(("t_last".to_owned(), Value::Integer(0)));
            responses.push(Response::success(summary));
            if self.streams.is_empty() {
                self.phase = if self.transaction.is_some() {
                    State::TxReady
                } else {
                    State::Ready
                };
            }
        } else {
            responses.push(Response::success(vec![(
                "has_more".to_owned(),
                Value::Boolean(true),
            )]));
        }
        Outcome {
            responses,
            close: false,
        }
    }

    /// Run one request on the engine off the async runtime, bounded by the
    /// same request timeout the HTTP handlers use.
    async fn execute(
        &self,
        request: EngineRequest,
    ) -> Result<CypherResponse, (&'static str, String)> {
        let engine = Arc::clone(&self.state.engine);
        let timeout = Duration::from_millis(self.state.config.request_timeout_ms);
        let joined = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let mut locked = engine
                    .lock()
                    .map_err(|_| "engine lock poisoned".to_owned())?;
                locked
                    .execute_request(request)
                    .map_err(|error| error.to_string())
            }),
        )
        .await;
        match joined {
            Err(_) => Err((
                code::TIMED_OUT,
                format!(
                    "the statement exceeded the {} ms request timeout",
                    timeout.as_millis()
                ),
            )),
            Ok(Err(join_error)) => {
                warn!(error = %join_error, "bolt execution task failed");
                Err((
                    code::EXECUTION_FAILED,
                    "the statement could not be executed".to_owned(),
                ))
            }
            Ok(Ok(Err(message))) => {
                // Engine errors are logged in full and reported generically:
                // storage paths and configuration never cross the wire.
                warn!(error = %message, "bolt execution rejected by the engine");
                Err((
                    code::EXECUTION_FAILED,
                    "the statement could not be executed".to_owned(),
                ))
            }
            Ok(Ok(Ok(response))) => Ok(response),
        }
    }

    fn encode_value(&self, value: &RecordValue) -> Value {
        match value {
            RecordValue::Null => Value::Null,
            RecordValue::Boolean(value) => Value::Boolean(*value),
            RecordValue::Integer(value) => Value::Integer(*value),
            RecordValue::Float(text) => text
                .parse::<f64>()
                .map_or_else(|_| Value::String(text.clone()), Value::Float),
            RecordValue::String(text) => Value::String(text.clone()),
            RecordValue::List(items) => {
                Value::List(items.iter().map(|item| self.encode_value(item)).collect())
            }
            RecordValue::Map(entries) => Value::Dictionary(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), self.encode_value(value)))
                    .collect(),
            ),
            RecordValue::Node(node) => {
                let mut fields = vec![
                    Value::Integer(legacy_id(&node.id)),
                    Value::List(node.labels.iter().cloned().map(Value::String).collect()),
                    self.encode_properties(&node.properties),
                ];
                if self.major >= 5 {
                    fields.push(Value::String(node.id.clone()));
                }
                Value::Structure { tag: b'N', fields }
            }
            RecordValue::Relationship(relationship) => {
                let mut fields = vec![
                    Value::Integer(legacy_id(&relationship.id)),
                    Value::Integer(legacy_id(&relationship.source_id)),
                    Value::Integer(legacy_id(&relationship.target_id)),
                    Value::String(relationship.rel_type.clone()),
                    self.encode_properties(&relationship.properties),
                ];
                if self.major >= 5 {
                    fields.push(Value::String(relationship.id.clone()));
                    fields.push(Value::String(relationship.source_id.clone()));
                    fields.push(Value::String(relationship.target_id.clone()));
                }
                Value::Structure { tag: b'R', fields }
            }
        }
    }

    fn encode_properties(&self, properties: &BTreeMap<String, RecordValue>) -> Value {
        Value::Dictionary(
            properties
                .iter()
                .map(|(key, value)| (key.clone(), self.encode_value(value)))
                .collect(),
        )
    }
}

/// Translate the runtime's rejection vocabulary into Bolt failure classes.
fn failure_from_response(response: &CypherResponse) -> (&'static str, String) {
    let first = response.validation_errors.first();
    let message = first
        .map(|error| error.message.clone())
        .or_else(|| response.warnings.first().cloned())
        .unwrap_or_else(|| "the statement was rejected".to_owned());
    let code = match first.map(|error| error.code.as_str()) {
        Some("WRITE_PERMISSION_REQUIRED" | "REQUEST_MODE_DISALLOWED") => code::FORBIDDEN,
        Some("QUERY_BUDGET_EXCEEDED" | "REQUEST_LIMIT_EXCEEDED") => code::BUDGET,
        Some(_) => code::SYNTAX,
        None => match response.status {
            CypherResponseStatus::ValidationFailed => code::SYNTAX,
            _ => code::EXECUTION_FAILED,
        },
    };
    (code, message)
}

/// PackStream parameters that Corrobore Cypher can bind.
fn to_cypher_value(value: &Value) -> Option<CypherValue> {
    Some(match value {
        Value::Null => CypherValue::Null,
        Value::Boolean(value) => CypherValue::Boolean(*value),
        Value::Integer(value) => CypherValue::Integer(*value),
        Value::Float(value) if value.is_finite() => CypherValue::Float(value.to_string()),
        Value::String(value) => CypherValue::String(value.clone()),
        Value::List(items) => CypherValue::List(
            items
                .iter()
                .map(to_cypher_value)
                .collect::<Option<Vec<_>>>()?,
        ),
        Value::Float(_) | Value::Bytes(_) | Value::Dictionary(_) | Value::Structure { .. } => {
            return None;
        }
    })
}

/// Drivers still expose an integer identity beside the element id. Corrobore
/// identifiers are strings, so the integer is a stable 63-bit FNV-1a digest of
/// the identifier; the element id carries the real one.
pub(super) fn legacy_id(id: &str) -> i64 {
    let mut hasher = std::hash::DefaultHasher::new();
    hasher.write(id.as_bytes());
    (hasher.finish() & (i64::MAX as u64)) as i64
}

fn bookmark() -> String {
    format!("corrobore:{}", Uuid::new_v4())
}

fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
