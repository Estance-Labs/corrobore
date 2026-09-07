// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! PostgreSQL wire-protocol conformance for the opt-in SQL listener (epic #90,
//! item #259).
//!
//! The client half is written here from the protocol specification, so the
//! contract under test is what `psql` and a PostgreSQL driver observe:
//! startup and cleartext authentication, the simple and extended query
//! protocols, typed row descriptions, catalog introspection, SQLSTATE errors,
//! statement-group transactions, and the standalone `sql` interface.
#![allow(clippy::unwrap_used)]

use std::{
    collections::HashMap,
    fs,
    net::TcpListener as StdTcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use corrobore_engine::{EngineRequest, EngineRequestMode};
use corrobore_http_server::{AppState, ServerConfig, sql::serve_sql};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const TOKEN: &str = "sql-contract-secret";
const PROTOCOL_3: i32 = 196_608;
const SSL_REQUEST: i32 = 80_877_103;

// Type OIDs a PostgreSQL client understands.
const OID_BOOL: i32 = 16;
const OID_INT8: i32 = 20;
const OID_TEXT: i32 = 25;
const OID_FLOAT8: i32 = 701;
const OID_JSONB: i32 = 3802;

fn state(extra: &[(&str, &str)]) -> AppState {
    let mut vars = HashMap::from([("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), TOKEN.to_owned())]);
    for (key, value) in extra {
        vars.insert((*key).to_owned(), (*value).to_owned());
    }
    AppState::new(ServerConfig::from_map(&vars).expect("configuration should parse"))
        .expect("state should initialize")
}

async fn start(state: AppState) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        serve_sql(listener, state, None)
            .await
            .expect("sql listener should run until asked to stop");
    });
    (port, handle)
}

/// Seed graph data through the engine, the way any other adapter would.
fn seed(state: &AppState, statements: &[&str]) {
    let mut engine = state.engine.lock().unwrap();
    for statement in statements {
        let response = engine
            .execute_request(EngineRequest::new(*statement, EngineRequestMode::Mutation))
            .unwrap();
        assert_eq!(
            response.status,
            shared_runtime::CypherResponseStatus::Success,
            "{statement}: {response:?}"
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Message {
    tag: u8,
    body: Vec<u8>,
}

struct Client {
    stream: TcpStream,
}

impl Client {
    async fn connect(port: u16) -> Self {
        Self {
            stream: TcpStream::connect(("127.0.0.1", port)).await.unwrap(),
        }
    }

    async fn startup(&mut self, user: &str, database: &str) {
        let mut body = Vec::new();
        body.extend_from_slice(&PROTOCOL_3.to_be_bytes());
        for (key, value) in [
            ("user", user),
            ("database", database),
            ("client_encoding", "UTF8"),
        ] {
            body.extend_from_slice(key.as_bytes());
            body.push(0);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        self.write_untyped(&body).await;
    }

    async fn write_untyped(&mut self, body: &[u8]) {
        let mut frame = Vec::new();
        frame.extend_from_slice(&((body.len() + 4) as i32).to_be_bytes());
        frame.extend_from_slice(body);
        self.stream.write_all(&frame).await.unwrap();
    }

    async fn write(&mut self, tag: u8, body: &[u8]) {
        let mut frame = vec![tag];
        frame.extend_from_slice(&((body.len() + 4) as i32).to_be_bytes());
        frame.extend_from_slice(body);
        self.stream.write_all(&frame).await.unwrap();
    }

    async fn read(&mut self) -> Option<Message> {
        let mut tag = [0u8; 1];
        if self.stream.read_exact(&mut tag).await.is_err() {
            return None;
        }
        let mut length = [0u8; 4];
        self.stream.read_exact(&mut length).await.unwrap();
        let length = i32::from_be_bytes(length) as usize - 4;
        let mut body = vec![0u8; length];
        self.stream.read_exact(&mut body).await.unwrap();
        Some(Message { tag: tag[0], body })
    }

    /// Read until ReadyForQuery, returning every message and the status byte.
    async fn read_until_ready(&mut self) -> (Vec<Message>, u8) {
        let mut messages = Vec::new();
        loop {
            let message = timeout(Duration::from_secs(10), self.read())
                .await
                .expect("server must answer within ten seconds")
                .expect("server closed the connection before ReadyForQuery");
            if message.tag == b'Z' {
                return (messages, message.body[0]);
            }
            messages.push(message);
        }
    }

    /// Authenticate with the cleartext password flow and return the parameter
    /// statuses the server announced.
    async fn login(port: u16, password: &str) -> (Self, HashMap<String, String>) {
        let mut client = Self::connect(port).await;
        client.startup("analyst", "corrobore").await;
        let auth = client.read().await.expect("authentication request");
        assert_eq!(auth.tag, b'R');
        assert_eq!(
            i32::from_be_bytes(auth.body[..4].try_into().unwrap()),
            3,
            "cleartext password"
        );
        let mut body = password.as_bytes().to_vec();
        body.push(0);
        client.write(b'p', &body).await;
        let (messages, status) = client.read_until_ready().await;
        assert_eq!(status, b'I');
        assert_eq!(messages[0].tag, b'R');
        assert_eq!(
            i32::from_be_bytes(messages[0].body[..4].try_into().unwrap()),
            0
        );
        let mut parameters = HashMap::new();
        for message in &messages {
            if message.tag == b'S' {
                let mut parts = message.body.split(|byte| *byte == 0);
                let key = String::from_utf8(parts.next().unwrap().to_vec()).unwrap();
                let value = String::from_utf8(parts.next().unwrap().to_vec()).unwrap();
                parameters.insert(key, value);
            }
        }
        assert!(
            messages.iter().any(|message| message.tag == b'K'),
            "BackendKeyData"
        );
        (client, parameters)
    }

    async fn simple(&mut self, sql: &str) -> (Vec<Message>, u8) {
        let mut body = sql.as_bytes().to_vec();
        body.push(0);
        self.write(b'Q', &body).await;
        self.read_until_ready().await
    }

    async fn sync(&mut self) -> (Vec<Message>, u8) {
        self.write(b'S', &[]).await;
        self.read_until_ready().await
    }
}

fn cstring(bytes: &[u8]) -> (String, &[u8]) {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap();
    (
        String::from_utf8(bytes[..end].to_vec()).unwrap(),
        &bytes[end + 1..],
    )
}

fn i16_at(bytes: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// (name, type oid) per column of a RowDescription.
fn columns(message: &Message) -> Vec<(String, i32)> {
    assert_eq!(message.tag, b'T');
    let count = i16_at(&message.body, 0) as usize;
    let mut rest = &message.body[2..];
    let mut columns = Vec::new();
    for _ in 0..count {
        let (name, tail) = cstring(rest);
        let oid = i32_at(tail, 6);
        columns.push((name, oid));
        rest = &tail[18..];
    }
    columns
}

/// Text cells of a DataRow; `None` is SQL NULL.
fn row(message: &Message) -> Vec<Option<String>> {
    assert_eq!(message.tag, b'D');
    let count = i16_at(&message.body, 0) as usize;
    let mut offset = 2;
    let mut cells = Vec::new();
    for _ in 0..count {
        let length = i32_at(&message.body, offset);
        offset += 4;
        if length < 0 {
            cells.push(None);
        } else {
            let end = offset + length as usize;
            cells.push(Some(
                String::from_utf8(message.body[offset..end].to_vec()).unwrap(),
            ));
            offset = end;
        }
    }
    cells
}

fn rows(messages: &[Message]) -> Vec<Vec<Option<String>>> {
    messages.iter().filter(|m| m.tag == b'D').map(row).collect()
}

fn command_tags(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|m| m.tag == b'C')
        .map(|m| cstring(&m.body).0)
        .collect()
}

/// (severity, sqlstate, message) of an ErrorResponse or NoticeResponse.
fn diagnostic(message: &Message) -> (String, String, String) {
    assert!(message.tag == b'E' || message.tag == b'N', "{message:?}");
    let mut fields = HashMap::new();
    let mut rest = &message.body[..];
    while !rest.is_empty() && rest[0] != 0 {
        let code = rest[0];
        let (value, tail) = cstring(&rest[1..]);
        fields.insert(code, value);
        rest = tail;
    }
    (
        fields.remove(&b'S').unwrap(),
        fields.remove(&b'C').unwrap(),
        fields.remove(&b'M').unwrap(),
    )
}

fn first_error(messages: &[Message]) -> (String, String, String) {
    diagnostic(
        messages
            .iter()
            .find(|m| m.tag == b'E')
            .expect("an ErrorResponse"),
    )
}

fn text(cells: &[Option<String>]) -> Vec<&str> {
    cells
        .iter()
        .map(|cell| cell.as_deref().unwrap_or("NULL"))
        .collect()
}

#[tokio::test]
async fn startup_authenticates_with_the_bearer_token_as_a_cleartext_password() {
    let (port, server) = start(state(&[])).await;

    let (_client, parameters) = Client::login(port, TOKEN).await;
    assert!(
        parameters["server_version"].contains("Corrobore"),
        "{parameters:?}"
    );
    assert_eq!(parameters["client_encoding"], "UTF8");
    assert_eq!(parameters["integer_datetimes"], "on");
    assert_eq!(parameters["standard_conforming_strings"], "on");

    let mut client = Client::connect(port).await;
    client.startup("analyst", "corrobore").await;
    assert_eq!(client.read().await.unwrap().tag, b'R');
    client.write(b'p', b"wrong\0").await;
    let error = client.read().await.expect("an ErrorResponse");
    let (severity, sqlstate, message) = diagnostic(&error);
    assert_eq!(severity, "FATAL");
    assert_eq!(sqlstate, "28P01");
    assert!(!message.contains(TOKEN));
    assert!(
        client.read().await.is_none(),
        "the connection closes after a failed login"
    );
    server.abort();
}

#[tokio::test]
async fn ssl_and_gss_requests_are_declined_without_tls_and_startup_continues() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::connect(port).await;
    client.write_untyped(&SSL_REQUEST.to_be_bytes()).await;
    let mut answer = [0u8; 1];
    client.stream.read_exact(&mut answer).await.unwrap();
    assert_eq!(answer, [b'N']);
    client.startup("analyst", "corrobore").await;
    assert_eq!(client.read().await.unwrap().tag, b'R');
    server.abort();
}

#[tokio::test]
async fn local_insecure_mode_skips_the_password() {
    let (port, server) = start(state(&[("CORROBORE_HTTP_AUTH_MODE", "local-insecure")])).await;
    let mut client = Client::connect(port).await;
    client.startup("analyst", "corrobore").await;
    let (messages, status) = client.read_until_ready().await;
    assert_eq!(status, b'I');
    assert_eq!(messages[0].tag, b'R');
    assert_eq!(
        i32_at(&messages[0].body, 0),
        0,
        "AuthenticationOk without a password round"
    );
    server.abort();
}

#[tokio::test]
async fn simple_query_answers_version_scalars_set_show_and_empty_statements() {
    let (port, server) = start(state(&[])).await;
    let (mut client, _) = Client::login(port, TOKEN).await;

    let (messages, status) = client.simple("SELECT version()").await;
    assert_eq!(status, b'I');
    assert_eq!(
        columns(&messages[0]),
        vec![("version".to_owned(), OID_TEXT)]
    );
    assert!(text(&rows(&messages)[0])[0].contains("Corrobore"));
    assert_eq!(command_tags(&messages), vec!["SELECT 1"]);

    let (messages, _) = client.simple("SELECT 1 AS one, 'x' AS s").await;
    let described = columns(&messages[0]);
    assert_eq!(described[0].0, "one");
    assert_eq!(described[1], ("s".to_owned(), OID_TEXT));
    assert_eq!(text(&rows(&messages)[0]), vec!["1", "x"]);

    let (messages, _) = client
        .simple("SET client_encoding TO 'UTF8'; SHOW server_version; SELECT current_user")
        .await;
    assert_eq!(command_tags(&messages), vec!["SET", "SHOW", "SELECT 1"]);

    let (messages, status) = client.simple("").await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].tag, b'I', "EmptyQueryResponse");
    assert_eq!(status, b'I');
    server.abort();
}

#[tokio::test]
async fn crud_round_trip_with_typed_columns_and_information_schema() {
    let (port, server) = start(state(&[])).await;
    let (mut client, _) = Client::login(port, TOKEN).await;

    let (messages, _) = client
        .simple(r#"INSERT INTO "Sensor" (name, reading, ratio, live, tags) VALUES ('north', 42, 0.5, true, ARRAY['a', 'b'])"#)
        .await;
    assert_eq!(command_tags(&messages), vec!["INSERT 0 1"]);

    let (messages, _) = client
        .simple(r#"SELECT s.name, s.reading, s.ratio, s.live, s.tags, s.id, s.missing FROM "Sensor" AS s WHERE s.reading > 40 ORDER BY s.name LIMIT 10"#)
        .await;
    assert_eq!(
        columns(&messages[0]),
        vec![
            ("name".to_owned(), OID_TEXT),
            ("reading".to_owned(), OID_INT8),
            ("ratio".to_owned(), OID_FLOAT8),
            ("live".to_owned(), OID_BOOL),
            ("tags".to_owned(), OID_JSONB),
            ("id".to_owned(), OID_TEXT),
            // A property no row has is NULL in every row and described as text.
            ("missing".to_owned(), OID_TEXT),
        ]
    );
    let result = rows(&messages);
    assert_eq!(result.len(), 1);
    assert_eq!(
        &text(&result[0])[..5],
        ["north", "42", "0.5", "t", r#"["a","b"]"#]
    );
    assert!(result[0][5].is_some());
    assert!(result[0][6].is_none());
    assert_eq!(command_tags(&messages), vec!["SELECT 1"]);

    let (messages, _) = client
        .simple(r#"UPDATE "Sensor" AS s SET reading = 43 WHERE s.name = 'north'"#)
        .await;
    assert_eq!(command_tags(&messages), vec!["UPDATE 1"]);
    let (messages, _) = client
        .simple(r#"SELECT s.reading FROM "Sensor" AS s"#)
        .await;
    assert_eq!(text(&rows(&messages)[0]), vec!["43"]);

    // information_schema reflects the labels and properties the graph holds.
    let (messages, _) = client
        .simple("SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name")
        .await;
    let tables: Vec<String> = rows(&messages)
        .into_iter()
        .map(|r| r[0].clone().unwrap())
        .collect();
    assert!(tables.contains(&"Sensor".to_owned()), "{tables:?}");
    assert!(tables.contains(&"nodes".to_owned()));
    assert!(tables.contains(&"relationships".to_owned()));
    let (messages, _) = client
        .simple("SELECT column_name, data_type FROM information_schema.columns WHERE table_name = 'Sensor' ORDER BY column_name")
        .await;
    let described: Vec<(String, String)> = rows(&messages)
        .into_iter()
        .map(|r| (r[0].clone().unwrap(), r[1].clone().unwrap()))
        .collect();
    assert!(
        described.contains(&("id".to_owned(), "text".to_owned())),
        "{described:?}"
    );
    assert!(described.contains(&("reading".to_owned(), "bigint".to_owned())));
    assert!(described.contains(&("live".to_owned(), "boolean".to_owned())));
    assert!(described.contains(&("tags".to_owned(), "jsonb".to_owned())));

    let (messages, _) = client
        .simple(r#"DELETE FROM "Sensor" AS s WHERE s.name = 'north'"#)
        .await;
    assert_eq!(command_tags(&messages), vec!["DELETE 1"]);
    let (messages, _) = client
        .simple(r#"SELECT count(*) AS total FROM "Sensor" AS s"#)
        .await;
    assert_eq!(columns(&messages[0]), vec![("total".to_owned(), OID_INT8)]);
    assert_eq!(text(&rows(&messages)[0]), vec!["0"]);
    server.abort();
}

#[tokio::test]
async fn joins_project_relationship_endpoints_and_aggregates() {
    let state = state(&[]);
    seed(
        &state,
        &[
            "CREATE (a:Actor {name: 'APT28', tier: 3})",
            "MATCH (a:Actor) CREATE (a)-[r:USES]->(m:Malware {name: 'X-Agent'})",
        ],
    );
    let (port, server) = start(state).await;
    let (mut client, _) = Client::login(port, TOKEN).await;

    let (messages, _) = client
        .simple(
            r#"SELECT a.name AS actor, r.type, r.source_id, r.target_id, m.name AS malware, a.id
               FROM "Actor" AS a JOIN "USES" AS r ON r.source_id = a.id JOIN "Malware" AS m ON r.target_id = m.id"#,
        )
        .await;
    let described = columns(&messages[0]);
    assert_eq!(
        described
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["actor", "type", "source_id", "target_id", "malware", "id"]
    );
    let result = rows(&messages);
    assert_eq!(result.len(), 1);
    let cells = text(&result[0]);
    assert_eq!(cells[0], "APT28");
    assert_eq!(cells[1], "USES");
    assert_eq!(cells[2], cells[5], "source_id is the actor's id");
    assert_eq!(cells[4], "X-Agent");

    let (messages, _) = client
        .simple(r#"SELECT count(*), max(a.tier) FROM "Actor" AS a"#)
        .await;
    assert_eq!(
        columns(&messages[0]),
        vec![("count".to_owned(), OID_INT8), ("max".to_owned(), OID_INT8)]
    );
    assert_eq!(text(&rows(&messages)[0]), vec!["1", "3"]);

    let (messages, _) = client.simple(r#"SELECT * FROM "Actor" AS a"#).await;
    let described = columns(&messages[0]);
    assert_eq!(described[0], ("id".to_owned(), OID_TEXT));
    assert_eq!(described[1], ("labels".to_owned(), OID_JSONB));
    assert_eq!(described[5], ("properties".to_owned(), OID_JSONB));
    let cells = rows(&messages);
    assert_eq!(cells[0][1].as_deref(), Some(r#"["Actor"]"#));
    assert_eq!(cells[0][5].as_deref(), Some(r#"{"name":"APT28","tier":3}"#));
    server.abort();
}

#[tokio::test]
async fn extended_protocol_binds_parameters_describes_and_suspends_portals() {
    let state = state(&[]);
    seed(
        &state,
        &[
            "CREATE (n:Reading {k: 1})",
            "CREATE (n:Reading {k: 2})",
            "CREATE (n:Reading {k: 3})",
        ],
    );
    let (port, server) = start(state).await;
    let (mut client, _) = Client::login(port, TOKEN).await;

    // Parse a named statement with two parameters of unspecified type.
    let mut parse = b"stmt\0".to_vec();
    parse.extend_from_slice(
        br#"SELECT n.k FROM "Reading" AS n WHERE n.k >= $1 ORDER BY n.k ASC LIMIT $2"#,
    );
    parse.push(0);
    parse.extend_from_slice(&2i16.to_be_bytes());
    parse.extend_from_slice(&0i32.to_be_bytes());
    parse.extend_from_slice(&0i32.to_be_bytes());
    client.write(b'P', &parse).await;
    client.write(b'D', b"Sstmt\0").await;
    let (messages, _) = client.sync().await;
    assert_eq!(messages[0].tag, b'1', "ParseComplete");
    assert_eq!(messages[1].tag, b't', "ParameterDescription");
    assert_eq!(i16_at(&messages[1].body, 0), 2);
    assert_eq!(columns(&messages[2]), vec![("k".to_owned(), OID_INT8)]);

    // Bind text parameters, describe the portal, execute with a row limit.
    let mut bind = b"portal\0stmt\0".to_vec();
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&2i16.to_be_bytes());
    for value in ["2", "10"] {
        bind.extend_from_slice(&(value.len() as i32).to_be_bytes());
        bind.extend_from_slice(value.as_bytes());
    }
    bind.extend_from_slice(&0i16.to_be_bytes());
    client.write(b'B', &bind).await;
    client.write(b'D', b"Pportal\0").await;
    let mut execute = b"portal\0".to_vec();
    execute.extend_from_slice(&1i32.to_be_bytes());
    client.write(b'E', &execute).await;
    client.write(b'H', &[]).await;
    let mut seen = Vec::new();
    loop {
        let message = client.read().await.unwrap();
        let done = message.tag == b's';
        seen.push(message);
        if done {
            break;
        }
    }
    assert_eq!(seen[0].tag, b'2', "BindComplete");
    assert_eq!(columns(&seen[1]), vec![("k".to_owned(), OID_INT8)]);
    assert_eq!(rows(&seen), vec![vec![Some("2".to_owned())]]);
    assert_eq!(
        seen.last().unwrap().tag,
        b's',
        "PortalSuspended after the row limit"
    );

    // The rest of the portal, then Sync.
    let mut execute = b"portal\0".to_vec();
    execute.extend_from_slice(&0i32.to_be_bytes());
    client.write(b'E', &execute).await;
    let (messages, status) = client.sync().await;
    assert_eq!(rows(&messages), vec![vec![Some("3".to_owned())]]);
    assert_eq!(command_tags(&messages), vec!["SELECT 2"]);
    assert_eq!(status, b'I');

    // Closing the statement and portal is acknowledged.
    client.write(b'C', b"Sstmt\0").await;
    let (messages, _) = client.sync().await;
    assert_eq!(messages[0].tag, b'3', "CloseComplete");

    // The unnamed statement with an int8 parameter in binary format.
    let mut parse = b"\0".to_vec();
    parse.extend_from_slice(br#"SELECT n.k FROM "Reading" AS n WHERE n.k = $1"#);
    parse.push(0);
    parse.extend_from_slice(&1i16.to_be_bytes());
    parse.extend_from_slice(&OID_INT8.to_be_bytes());
    client.write(b'P', &parse).await;
    let mut bind = b"\0\0".to_vec();
    bind.extend_from_slice(&1i16.to_be_bytes());
    bind.extend_from_slice(&1i16.to_be_bytes());
    bind.extend_from_slice(&1i16.to_be_bytes());
    bind.extend_from_slice(&8i32.to_be_bytes());
    bind.extend_from_slice(&3i64.to_be_bytes());
    bind.extend_from_slice(&0i16.to_be_bytes());
    client.write(b'B', &bind).await;
    let mut execute = b"\0".to_vec();
    execute.extend_from_slice(&0i32.to_be_bytes());
    client.write(b'E', &execute).await;
    let (messages, _) = client.sync().await;
    assert_eq!(rows(&messages), vec![vec![Some("3".to_owned())]]);

    // A Bind that misses a parameter fails at bind time with a SQLSTATE and
    // the rest of the batch is skipped until Sync.
    let mut bind = b"\0\0".to_vec();
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&0i16.to_be_bytes());
    bind.extend_from_slice(&0i16.to_be_bytes());
    client.write(b'B', &bind).await;
    client.write(b'E', &execute).await;
    let (messages, status) = client.sync().await;
    assert_eq!(first_error(&messages).1, "08P01");
    assert_eq!(status, b'I');
    server.abort();
}

#[tokio::test]
async fn errors_carry_sqlstates_and_transactions_are_statement_groups() {
    let (port, server) = start(state(&[])).await;
    let (mut client, _) = Client::login(port, TOKEN).await;

    let (messages, status) = client.simple("SELEKT 1").await;
    let (severity, sqlstate, _) = first_error(&messages);
    assert_eq!(severity, "ERROR");
    assert_eq!(sqlstate, "42601");
    assert_eq!(status, b'I');

    let (messages, _) = client
        .simple("WITH RECURSIVE t AS (SELECT 1) SELECT * FROM t")
        .await;
    assert_eq!(first_error(&messages).1, "0A000");

    // Statements after a failing one in the same simple query are skipped.
    let (messages, _) = client
        .simple(r#"SELEKT 1; INSERT INTO "Skipped" (k) VALUES (1)"#)
        .await;
    assert!(command_tags(&messages).is_empty());
    let (messages, _) = client
        .simple(r#"SELECT count(*) FROM "Skipped" AS s"#)
        .await;
    assert_eq!(text(&rows(&messages)[0]), vec!["0"]);

    // A read-only group refuses writes with insufficient_privilege.
    let (messages, status) = client.simple("BEGIN READ ONLY").await;
    assert_eq!(command_tags(&messages), vec!["BEGIN"]);
    assert_eq!(status, b'T');
    let (messages, status) = client.simple(r#"INSERT INTO "Tx" (k) VALUES (1)"#).await;
    assert_eq!(first_error(&messages).1, "42501");
    assert_eq!(status, b'E', "the group is failed until it ends");
    let (messages, status) = client.simple("SELECT 1").await;
    assert_eq!(
        first_error(&messages).1,
        "25P02",
        "in_failed_sql_transaction"
    );
    assert_eq!(status, b'E');
    let (messages, status) = client.simple("ROLLBACK").await;
    assert_eq!(command_tags(&messages), vec!["ROLLBACK"]);
    assert_eq!(status, b'I');

    // A write group applies each statement durably; COMMIT acknowledges.
    client.simple("BEGIN").await;
    client.simple(r#"INSERT INTO "Tx" (k) VALUES (1)"#).await;
    client.simple(r#"INSERT INTO "Tx" (k) VALUES (2)"#).await;
    let (messages, status) = client.simple("COMMIT").await;
    assert_eq!(command_tags(&messages), vec!["COMMIT"]);
    assert_eq!(status, b'I');

    // ROLLBACK after an applied write ends the group, keeps the client
    // usable, and says what happened in a WARNING notice.
    client.simple("BEGIN").await;
    client.simple(r#"INSERT INTO "Tx" (k) VALUES (3)"#).await;
    let (messages, status) = client.simple("ROLLBACK").await;
    let notice = messages
        .iter()
        .find(|m| m.tag == b'N')
        .expect("a NoticeResponse");
    let (severity, sqlstate, message) = diagnostic(notice);
    assert_eq!(severity, "WARNING");
    assert_eq!(sqlstate, "01000");
    assert!(message.contains("already applied"), "{message}");
    assert_eq!(command_tags(&messages), vec!["ROLLBACK"]);
    assert_eq!(status, b'I');
    let (messages, _) = client.simple(r#"SELECT count(*) FROM "Tx" AS t"#).await;
    assert_eq!(text(&rows(&messages)[0]), vec!["3"]);

    // COMMIT outside a transaction is a warning, as in PostgreSQL.
    let (messages, status) = client.simple("COMMIT").await;
    assert_eq!(
        diagnostic(messages.iter().find(|m| m.tag == b'N').unwrap()).1,
        "25P01"
    );
    assert_eq!(command_tags(&messages), vec!["COMMIT"]);
    assert_eq!(status, b'I');
    server.abort();
}

#[tokio::test]
async fn terminate_closes_and_draining_refuses_new_statements() {
    let state = state(&[]);
    let lifecycle = Arc::clone(&state.lifecycle);
    let (port, server) = start(state).await;
    let (mut client, _) = Client::login(port, TOKEN).await;
    client.write(b'X', &[]).await;
    assert!(
        client.read().await.is_none(),
        "Terminate closes the connection"
    );

    let (mut client, _) = Client::login(port, TOKEN).await;
    lifecycle.begin_draining();
    let finished = timeout(Duration::from_secs(5), server).await;
    assert!(
        matches!(finished, Ok(Ok(()))),
        "the listener stops when draining"
    );
    let (messages, _) = client.simple("SELECT 1").await;
    assert_eq!(first_error(&messages).1, "57P01", "admin_shutdown");
}

#[test]
fn server_config_parses_sql_settings_with_safe_defaults() {
    let config = ServerConfig::from_map(&HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        TOKEN.to_owned(),
    )]))
    .unwrap();
    assert_eq!(config.sql_port, 5432);
    assert_eq!(config.sql_max_connections, 64);

    let config = ServerConfig::from_map(&HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), TOKEN.to_owned()),
        ("CORROBORE_SQL_PORT".to_owned(), "15432".to_owned()),
        ("CORROBORE_SQL_MAX_CONNECTIONS".to_owned(), "8".to_owned()),
    ]))
    .unwrap();
    assert_eq!(config.sql_port, 15432);
    assert_eq!(config.sql_max_connections, 8);

    for (name, value) in [
        ("CORROBORE_SQL_PORT", "five"),
        ("CORROBORE_SQL_MAX_CONNECTIONS", "0"),
    ] {
        let error = ServerConfig::from_map(&HashMap::from([
            ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), TOKEN.to_owned()),
            (name.to_owned(), value.to_owned()),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains(name), "{error}");
    }
}

// --- standalone binary -------------------------------------------------------

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-sql-{name}-{suffix}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn reserve_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn_standalone(
    directory: &std::path::Path,
    interfaces: &str,
    http_port: u16,
    sql_port: u16,
) -> Child {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
        .args([
            "server",
            "start",
            "--host",
            "127.0.0.1",
            "--port",
            &http_port.to_string(),
            "--auth-token",
            TOKEN,
            "--interfaces",
            interfaces,
            "--sql-port",
            &sql_port.to_string(),
            "--data-dir",
            directory.join("runtime").to_str().unwrap(),
            "--log-dir",
            directory.join("logs").to_str().unwrap(),
        ])
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for(port: u16) -> bool {
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn standalone_start_opens_the_sql_listener_only_when_the_interface_is_enabled() {
    let directory = temp_dir("standalone");
    let http_port = reserve_port();
    let sql_port = reserve_port();
    let mut child = spawn_standalone(&directory, "http,sql", http_port, sql_port);
    assert!(wait_for(http_port));
    assert!(wait_for(sql_port), "SQL listener should open beside HTTP");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (mut client, _) = Client::login(sql_port, TOKEN).await;
        let (messages, _) = client.simple("SELECT 1 AS one").await;
        assert_eq!(text(&rows(&messages)[0]), vec!["1"]);
    });
    child.kill().unwrap();
    child.wait().unwrap();

    let directory = temp_dir("http-only");
    let http_port = reserve_port();
    let sql_port = reserve_port();
    let mut child = spawn_standalone(&directory, "http", http_port, sql_port);
    assert!(wait_for(http_port));
    std::thread::sleep(Duration::from_millis(300));
    assert!(std::net::TcpStream::connect(("127.0.0.1", sql_port)).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn validate_config_accepts_the_sql_section_and_prints_it() {
    let directory = temp_dir("validate");
    let config_path = directory.join("corrobore.toml");
    fs::write(
        &config_path,
        format!(
            r#"
[server]
auth_token = "{TOKEN}"

[interfaces]
enabled = ["http", "sql"]

[sql]
port = 15433
max_connections = 9
"#
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corrobore"))
        .args([
            "server",
            "validate-config",
            "--config",
            config_path.to_str().unwrap(),
            "--print-effective",
        ])
        .env_clear()
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(r#"interfaces.enabled = ["http", "sql"]"#),
        "{stdout}"
    );
    assert!(stdout.contains("sql.port = 15433"), "{stdout}");
    assert!(stdout.contains("sql.max_connections = 9"), "{stdout}");
}
