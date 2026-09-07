// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Bolt protocol conformance for the opt-in listener (epic #12, item #258).
//!
//! The client half of every test is written here from the Bolt specification
//! rather than borrowed from the server, so the contract under test is the wire
//! behaviour a Neo4j driver observes: handshake, PackStream framing,
//! authentication, streaming, failures, transactions and routing.
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

use corrobore_http_server::{
    AppState, ServerConfig,
    bolt::{
        packstream::{Value, decode, encode},
        serve_bolt,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const TOKEN: &str = "bolt-contract-secret";
const BOLT_MAGIC: [u8; 4] = [0x60, 0x60, 0xB0, 0x17];
/// A read the bounded parser accepts on an empty graph; it yields one row `[0]`.
const PING: &str = "MATCH (n:Ping) RETURN count(n)";

// Message signatures from the Bolt specification.
const HELLO: u8 = 0x01;
const GOODBYE: u8 = 0x02;
const RESET: u8 = 0x0F;
const RUN: u8 = 0x10;
const BEGIN: u8 = 0x11;
const COMMIT: u8 = 0x12;
const ROLLBACK: u8 = 0x13;
const DISCARD: u8 = 0x2F;
const PULL: u8 = 0x3F;
const ROUTE: u8 = 0x66;
const LOGON: u8 = 0x6A;
const SUCCESS: u8 = 0x70;
const RECORD: u8 = 0x71;
const IGNORED: u8 = 0x7E;
const FAILURE: u8 = 0x7F;

/// Offer `[major.minor .. major.(minor-range)]` the way a driver does.
const fn version(major: u8, minor: u8, range: u8) -> u32 {
    ((range as u32) << 16) | ((minor as u32) << 8) | (major as u32)
}

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
        serve_bolt(listener, state, None)
            .await
            .expect("bolt listener should run until asked to stop");
    });
    (port, handle)
}

struct Client {
    stream: TcpStream,
}

impl Client {
    async fn connect(port: u16, offers: [u32; 4]) -> (Self, u32) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut handshake = BOLT_MAGIC.to_vec();
        for offer in offers {
            handshake.extend_from_slice(&offer.to_be_bytes());
        }
        stream.write_all(&handshake).await.unwrap();
        let mut chosen = [0u8; 4];
        stream.read_exact(&mut chosen).await.unwrap();
        (Self { stream }, u32::from_be_bytes(chosen))
    }

    /// Negotiate Bolt 5.4 and authenticate with HELLO + LOGON.
    async fn ready(port: u16) -> Self {
        let (mut client, negotiated) = Self::connect(port, [version(5, 4, 4), 0, 0, 0]).await;
        assert_eq!(negotiated, version(5, 4, 0), "server must pick 5.4");
        let (tag, fields) = client
            .call(HELLO, vec![dict([("user_agent", str("contract/1.0"))])])
            .await;
        assert_eq!(tag, SUCCESS, "HELLO must succeed: {fields:?}");
        let (tag, fields) = client
            .call(
                LOGON,
                vec![dict([
                    ("scheme", str("basic")),
                    ("principal", str("analyst")),
                    ("credentials", str(TOKEN)),
                ])],
            )
            .await;
        assert_eq!(tag, SUCCESS, "LOGON must succeed: {fields:?}");
        client
    }

    async fn send(&mut self, tag: u8, fields: Vec<Value>) {
        let mut payload = Vec::new();
        encode(&Value::Structure { tag, fields }, &mut payload);
        let mut framed = Vec::new();
        for chunk in payload.chunks(0xFFFF) {
            framed.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
            framed.extend_from_slice(chunk);
        }
        framed.extend_from_slice(&[0, 0]);
        self.stream.write_all(&framed).await.unwrap();
    }

    async fn recv(&mut self) -> (u8, Vec<Value>) {
        self.try_recv()
            .await
            .expect("server closed the connection before responding")
    }

    async fn try_recv(&mut self) -> Option<(u8, Vec<Value>)> {
        let mut payload = Vec::new();
        loop {
            let mut header = [0u8; 2];
            if self.stream.read_exact(&mut header).await.is_err() {
                return None;
            }
            let length = u16::from_be_bytes(header) as usize;
            if length == 0 {
                if payload.is_empty() {
                    // A leading no-op chunk keeps the connection alive.
                    continue;
                }
                break;
            }
            let start = payload.len();
            payload.resize(start + length, 0);
            self.stream.read_exact(&mut payload[start..]).await.unwrap();
        }
        let (value, consumed) = decode(&payload).expect("response must be valid PackStream");
        assert_eq!(
            consumed,
            payload.len(),
            "response must be exactly one message"
        );
        match value {
            Value::Structure { tag, fields } => Some((tag, fields)),
            other => panic!("response must be a structure, got {other:?}"),
        }
    }

    async fn call(&mut self, tag: u8, fields: Vec<Value>) -> (u8, Vec<Value>) {
        self.send(tag, fields).await;
        self.recv().await
    }

    /// RUN + PULL(-1); returns the RUN metadata, the records and the summary.
    async fn run(&mut self, query: &str, params: Value) -> (Value, Vec<Vec<Value>>, Value) {
        let (tag, fields) = self.call(RUN, vec![str(query), params, dict([])]).await;
        assert_eq!(tag, SUCCESS, "RUN {query} must succeed: {fields:?}");
        let run_meta = fields.into_iter().next().unwrap();
        let (records, summary) = self.pull(-1).await;
        (run_meta, records, summary)
    }

    async fn pull(&mut self, n: i64) -> (Vec<Vec<Value>>, Value) {
        self.send(PULL, vec![dict([("n", Value::Integer(n))])])
            .await;
        let mut records = Vec::new();
        loop {
            let (tag, fields) = self.recv().await;
            match tag {
                RECORD => {
                    let Value::List(values) = fields.into_iter().next().unwrap() else {
                        panic!("RECORD carries a list");
                    };
                    records.push(values);
                }
                SUCCESS => return (records, fields.into_iter().next().unwrap()),
                other => panic!("unexpected response {other:#x} to PULL: {fields:?}"),
            }
        }
    }
}

fn str(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn dict<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Dictionary(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Dictionary(entries) => entries
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value),
        _ => None,
    }
}

fn text(value: &Value) -> &str {
    match value {
        Value::String(text) => text,
        other => panic!("expected a string, got {other:?}"),
    }
}

fn failure_code(fields: &[Value]) -> String {
    text(get(&fields[0], "code").expect("FAILURE carries a code")).to_owned()
}

#[tokio::test]
async fn handshake_picks_the_highest_supported_offer_and_refuses_unknown_versions() {
    let (port, server) = start(state(&[])).await;

    // A range offer: 5.8 down to 5.0 negotiates the highest version served.
    let (_client, negotiated) = Client::connect(port, [version(5, 8, 8), 0, 0, 0]).await;
    assert_eq!(negotiated, version(5, 8, 0));

    // A driver 6 style manifest offer (0x000001FF) degrades to the other slots.
    let (_client, negotiated) =
        Client::connect(port, [0x0000_01FF, version(5, 4, 4), version(4, 4, 0), 0]).await;
    assert_eq!(negotiated, version(5, 4, 0));

    // Bolt 4.4 alone is accepted.
    let (_client, negotiated) = Client::connect(port, [version(4, 4, 0), 0, 0, 0]).await;
    assert_eq!(negotiated, version(4, 4, 0));

    // Only unsupported versions: the server answers zero and closes.
    let (mut client, negotiated) =
        Client::connect(port, [version(3, 0, 0), version(1, 0, 0), 0, 0]).await;
    assert_eq!(negotiated, 0);
    let mut byte = [0u8; 1];
    assert_eq!(
        client.stream.read(&mut byte).await.unwrap(),
        0,
        "connection must be closed"
    );

    // Garbage instead of the magic preamble is refused without a response.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(&[
            0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ])
        .await
        .unwrap();
    let mut buffer = [0u8; 4];
    assert!(
        matches!(stream.read(&mut buffer).await, Ok(0) | Err(_)),
        "a non-Bolt client gets no protocol answer"
    );
    server.abort();
}

#[tokio::test]
async fn hello_then_logon_authenticates_and_returns_typed_records() {
    let (port, server) = start(state(&[])).await;
    let (mut client, _) = Client::connect(port, [version(5, 4, 4), 0, 0, 0]).await;

    let (tag, fields) = client
        .call(
            HELLO,
            vec![dict([
                ("user_agent", str("contract/1.0")),
                ("bolt_agent", dict([("product", str("contract/1.0"))])),
            ])],
        )
        .await;
    assert_eq!(tag, SUCCESS);
    let meta = &fields[0];
    assert!(text(get(meta, "server").unwrap()).starts_with("Corrobore/"));
    assert!(get(meta, "connection_id").is_some());
    // Bolt 5.1+: HELLO carries no credentials, so the session is not ready yet.
    let (tag, fields) = client.call(RUN, vec![str(PING), dict([]), dict([])]).await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Security.Unauthorized"
    );
    let (tag, _) = client.call(RESET, vec![]).await;
    assert_eq!(tag, SUCCESS);

    let (tag, _) = client
        .call(
            LOGON,
            vec![dict([
                ("scheme", str("basic")),
                ("principal", str("analyst")),
                ("credentials", str(TOKEN)),
            ])],
        )
        .await;
    assert_eq!(tag, SUCCESS);

    let (run_meta, records, summary) = client
        .run(
            "CREATE (n:Sensor {name: 'north', reading: 42, ratio: 0.5, live: true, tags: ['a', 'b']}) RETURN n",
            dict([]),
        )
        .await;
    assert_eq!(
        get(&run_meta, "fields").unwrap(),
        &Value::List(vec![str("n")]),
        "RUN advertises the ordered columns"
    );
    assert_eq!(records.len(), 1);
    let Value::Structure { tag: b'N', fields } = &records[0][0] else {
        panic!("a whole node is a Node structure, got {:?}", records[0][0]);
    };
    // Bolt 5: id, labels, properties, element_id.
    assert_eq!(fields.len(), 4);
    assert!(matches!(fields[0], Value::Integer(_)));
    assert_eq!(fields[1], Value::List(vec![str("Sensor")]));
    let properties = &fields[2];
    assert_eq!(get(properties, "name"), Some(&str("north")));
    assert_eq!(get(properties, "reading"), Some(&Value::Integer(42)));
    assert_eq!(get(properties, "ratio"), Some(&Value::Float(0.5)));
    assert_eq!(get(properties, "live"), Some(&Value::Boolean(true)));
    assert_eq!(
        get(properties, "tags"),
        Some(&Value::List(vec![str("a"), str("b")]))
    );
    assert!(text(&fields[3]).starts_with("node--") || !text(&fields[3]).is_empty());
    // A statement that wrote and returned rows is a read-write result; the
    // database name and timing travel with every summary.
    assert_eq!(get(&summary, "type"), Some(&str("rw")), "{summary:?}");
    assert_eq!(get(&summary, "db"), Some(&str("corrobore")));
    assert!(get(&summary, "t_last").is_some());

    let (run_meta, records, summary) = client
        .run(
            "MATCH (n:Sensor) RETURN n.name, n.reading, n.ratio, n.live LIMIT 5",
            dict([]),
        )
        .await;
    assert_eq!(
        get(&run_meta, "fields").unwrap(),
        &Value::List(vec![
            str("n.name"),
            str("n.reading"),
            str("n.ratio"),
            str("n.live")
        ])
    );
    assert_eq!(
        records,
        vec![vec![
            str("north"),
            Value::Integer(42),
            Value::Float(0.5),
            Value::Boolean(true)
        ]]
    );
    assert_eq!(get(&summary, "type"), Some(&str("r")));
    assert!(get(&summary, "has_more").is_none());
    server.abort();
}

#[tokio::test]
async fn bolt_4_4_hello_carries_credentials_and_legacy_structures() {
    let (port, server) = start(state(&[])).await;
    let (mut client, negotiated) = Client::connect(port, [version(4, 4, 0), 0, 0, 0]).await;
    assert_eq!(negotiated, version(4, 4, 0));

    let (tag, fields) = client
        .call(
            HELLO,
            vec![dict([
                ("user_agent", str("legacy/4.4")),
                ("scheme", str("basic")),
                ("principal", str("analyst")),
                ("credentials", str(TOKEN)),
            ])],
        )
        .await;
    assert_eq!(tag, SUCCESS, "4.4 HELLO authenticates inline: {fields:?}");

    client
        .run("CREATE (a:Actor {name: 'APT28'})", dict([]))
        .await;
    client
        .run(
            "MATCH (a:Actor) CREATE (a)-[r:USES]->(m:Malware {name: 'X-Agent'})",
            dict([]),
        )
        .await;
    let (_, records, _) = client
        .run(
            "MATCH (a:Actor)-[r:USES]->(m:Malware) RETURN a, r, m LIMIT 1",
            dict([]),
        )
        .await;
    let row = &records[0];
    let Value::Structure { tag: b'N', fields } = &row[0] else {
        panic!("node expected");
    };
    // Bolt 4.4: id, labels, properties only.
    assert_eq!(fields.len(), 3);
    let Value::Structure { tag: b'R', fields } = &row[1] else {
        panic!("relationship expected, got {:?}", row[1]);
    };
    // Bolt 4.4: id, start, end, type, properties.
    assert_eq!(fields.len(), 5);
    assert_eq!(fields[3], str("USES"));
    let Value::Structure { tag: b'N', .. } = &row[2] else {
        panic!("node expected");
    };
    server.abort();
}

#[tokio::test]
async fn bolt_5_relationships_carry_element_identifiers() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::ready(port).await;
    client
        .run("CREATE (a:Actor {name: 'APT28'})", dict([]))
        .await;
    client
        .run(
            "MATCH (a:Actor) CREATE (a)-[r:USES]->(m:Malware {name: 'X-Agent'})",
            dict([]),
        )
        .await;
    let (_, records, _) = client
        .run(
            "MATCH (a:Actor)-[r:USES]->(m:Malware) RETURN a, r, m LIMIT 1",
            dict([]),
        )
        .await;
    let row = &records[0];
    let Value::Structure {
        tag: b'N',
        fields: actor,
    } = &row[0]
    else {
        panic!("node expected");
    };
    let Value::Structure {
        tag: b'R',
        fields: uses,
    } = &row[1]
    else {
        panic!("relationship expected");
    };
    let Value::Structure {
        tag: b'N',
        fields: malware,
    } = &row[2]
    else {
        panic!("node expected");
    };
    // Bolt 5: id, start, end, type, properties, element_id, start_element_id,
    // end_element_id; the legacy integer ids and the element ids agree.
    assert_eq!(uses.len(), 8);
    assert_eq!(uses[1], actor[0]);
    assert_eq!(uses[2], malware[0]);
    assert_eq!(uses[3], str("USES"));
    assert_eq!(uses[6], actor[3]);
    assert_eq!(uses[7], malware[3]);
    server.abort();
}

#[tokio::test]
async fn bearer_scheme_is_accepted_and_bad_credentials_close_the_connection() {
    let (port, server) = start(state(&[])).await;

    let (mut client, _) = Client::connect(port, [version(5, 4, 4), 0, 0, 0]).await;
    client
        .call(HELLO, vec![dict([("user_agent", str("c"))])])
        .await;
    let (tag, _) = client
        .call(
            LOGON,
            vec![dict([
                ("scheme", str("bearer")),
                ("credentials", str(TOKEN)),
            ])],
        )
        .await;
    assert_eq!(tag, SUCCESS);

    let (mut client, _) = Client::connect(port, [version(5, 4, 4), 0, 0, 0]).await;
    client
        .call(HELLO, vec![dict([("user_agent", str("c"))])])
        .await;
    let (tag, fields) = client
        .call(
            LOGON,
            vec![dict([
                ("scheme", str("basic")),
                ("principal", str("analyst")),
                ("credentials", str("wrong")),
            ])],
        )
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Security.Unauthorized"
    );
    let message = text(get(&fields[0], "message").unwrap());
    assert!(!message.contains(TOKEN), "a failure never echoes a secret");
    assert!(
        client.try_recv().await.is_none(),
        "the server closes after a failed login"
    );

    // `none` is refused when authentication is required.
    let (mut client, _) = Client::connect(port, [version(5, 4, 4), 0, 0, 0]).await;
    client
        .call(HELLO, vec![dict([("user_agent", str("c"))])])
        .await;
    let (tag, fields) = client
        .call(LOGON, vec![dict([("scheme", str("none"))])])
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Security.Unauthorized"
    );
    server.abort();
}

#[tokio::test]
async fn bolt_5_7_failures_carry_the_gql_fields_beside_the_classic_code() {
    let (port, server) = start(state(&[])).await;
    let (mut client, negotiated) = Client::connect(port, [version(5, 8, 0), 0, 0, 0]).await;
    assert_eq!(negotiated, version(5, 8, 0));
    client
        .call(HELLO, vec![dict([("user_agent", str("c"))])])
        .await;
    let (tag, fields) = client
        .call(
            LOGON,
            vec![dict([
                ("scheme", str("basic")),
                ("principal", str("a")),
                ("credentials", str("bad")),
            ])],
        )
        .await;
    assert_eq!(tag, FAILURE);
    let metadata = &fields[0];
    // Drivers on 5.7+ read `neo4j_code` and `gql_status`; older ones read `code`.
    assert_eq!(
        get(metadata, "code"),
        Some(&str("Neo.ClientError.Security.Unauthorized"))
    );
    assert_eq!(
        get(metadata, "neo4j_code"),
        Some(&str("Neo.ClientError.Security.Unauthorized"))
    );
    assert_eq!(get(metadata, "gql_status"), Some(&str("42N42")));
    assert!(get(metadata, "diagnostic_record").is_some());
    server.abort();

    // On 5.6 and below the classic shape is sent alone.
    let (port, server) = start(state(&[])).await;
    let (mut client, _) = Client::connect(port, [version(5, 6, 0), 0, 0, 0]).await;
    client
        .call(HELLO, vec![dict([("user_agent", str("c"))])])
        .await;
    let (tag, fields) = client
        .call(
            LOGON,
            vec![dict([
                ("scheme", str("basic")),
                ("principal", str("a")),
                ("credentials", str("bad")),
            ])],
        )
        .await;
    assert_eq!(tag, FAILURE);
    assert!(get(&fields[0], "neo4j_code").is_none());
    server.abort();
}

#[tokio::test]
async fn local_insecure_mode_accepts_the_none_scheme_on_loopback() {
    let (port, server) = start(state(&[("CORROBORE_HTTP_AUTH_MODE", "local-insecure")])).await;
    let (mut client, _) = Client::connect(port, [version(5, 4, 4), 0, 0, 0]).await;
    client
        .call(HELLO, vec![dict([("user_agent", str("c"))])])
        .await;
    let (tag, _) = client
        .call(LOGON, vec![dict([("scheme", str("none"))])])
        .await;
    assert_eq!(tag, SUCCESS);
    server.abort();
}

#[tokio::test]
async fn pull_streams_records_in_bounded_batches() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::ready(port).await;
    for index in 0..5 {
        client
            .run(&format!("CREATE (n:Streamed {{k: {index}}})"), dict([]))
            .await;
    }

    let (tag, fields) = client
        .call(
            RUN,
            vec![
                str("MATCH (n:Streamed) RETURN n.k ORDER BY n.k ASC LIMIT 10"),
                dict([]),
                dict([]),
            ],
        )
        .await;
    assert_eq!(tag, SUCCESS, "{fields:?}");

    let (batch, summary) = client.pull(2).await;
    assert_eq!(
        batch,
        vec![vec![Value::Integer(0)], vec![Value::Integer(1)]]
    );
    assert_eq!(get(&summary, "has_more"), Some(&Value::Boolean(true)));

    let (batch, summary) = client.pull(2).await;
    assert_eq!(
        batch,
        vec![vec![Value::Integer(2)], vec![Value::Integer(3)]]
    );
    assert_eq!(get(&summary, "has_more"), Some(&Value::Boolean(true)));

    let (batch, summary) = client.pull(-1).await;
    assert_eq!(batch, vec![vec![Value::Integer(4)]]);
    assert!(
        get(&summary, "has_more").is_none(),
        "the final summary ends the stream"
    );
    assert_eq!(get(&summary, "type"), Some(&str("r")));

    // DISCARD drops the rest of a stream and leaves the session ready.
    let (tag, _) = client
        .call(
            RUN,
            vec![
                str("MATCH (n:Streamed) RETURN n.k LIMIT 10"),
                dict([]),
                dict([]),
            ],
        )
        .await;
    assert_eq!(tag, SUCCESS);
    let (tag, fields) = client
        .call(DISCARD, vec![dict([("n", Value::Integer(-1))])])
        .await;
    assert_eq!(tag, SUCCESS, "{fields:?}");
    let (_, records, _) = client.run(PING, dict([])).await;
    assert_eq!(records, vec![vec![Value::Integer(0)]]);
    server.abort();
}

#[tokio::test]
async fn failures_map_to_stable_codes_and_reset_recovers_the_session() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::ready(port).await;

    let (tag, fields) = client
        .call(RUN, vec![str("THIS IS NOT CYPHER"), dict([]), dict([])])
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Statement.SyntaxError"
    );
    // Until RESET, further requests are IGNORED rather than executed.
    let (tag, _) = client
        .call(PULL, vec![dict([("n", Value::Integer(-1))])])
        .await;
    assert_eq!(tag, IGNORED);
    let (tag, _) = client.call(RESET, vec![]).await;
    assert_eq!(tag, SUCCESS);
    let (_, records, _) = client.run(PING, dict([])).await;
    assert_eq!(records, vec![vec![Value::Integer(0)]]);

    // Unsupported Cypher is a syntax-class failure as well.
    let (tag, fields) = client
        .call(
            RUN,
            vec![str("MATCH (n) DETACH DELETE n"), dict([]), dict([])],
        )
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Statement.SyntaxError"
    );
    client.call(RESET, vec![]).await;

    // A write inside a read transaction is refused by the runtime policy.
    let (tag, _) = client.call(BEGIN, vec![dict([("mode", str("r"))])]).await;
    assert_eq!(tag, SUCCESS);
    let (tag, fields) = client
        .call(
            RUN,
            vec![str("CREATE (n:Forbidden {k: 1})"), dict([]), dict([])],
        )
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(failure_code(&fields), "Neo.ClientError.Security.Forbidden");
    client.call(RESET, vec![]).await;
    let (_, records, _) = client
        .run("MATCH (n:Forbidden) RETURN count(n)", dict([]))
        .await;
    assert_eq!(
        records,
        vec![vec![Value::Integer(0)]],
        "the refused write left no effect"
    );

    // A PULL with nothing to pull is a request error, not a crash.
    let (tag, fields) = client
        .call(PULL, vec![dict([("n", Value::Integer(-1))])])
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(failure_code(&fields), "Neo.ClientError.Request.Invalid");
    server.abort();
}

#[tokio::test]
async fn explicit_transactions_are_statement_groups_over_atomic_requests() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::ready(port).await;

    let (tag, _) = client.call(BEGIN, vec![dict([])]).await;
    assert_eq!(tag, SUCCESS);
    let (_, _, summary) = client.run("CREATE (n:Tx {k: 1})", dict([])).await;
    assert_eq!(get(&summary, "type"), Some(&str("w")));
    let Some(stats) = get(&summary, "stats") else {
        panic!("a write summary carries counters: {summary:?}");
    };
    assert_eq!(get(stats, "nodes-created"), Some(&Value::Integer(1)));
    client.run("CREATE (n:Tx {k: 2})", dict([])).await;
    let (tag, fields) = client.call(COMMIT, vec![]).await;
    assert_eq!(tag, SUCCESS);
    assert!(
        get(&fields[0], "bookmark").is_some(),
        "COMMIT returns a bookmark: {fields:?}"
    );

    // A read-only group rolls back trivially.
    let (tag, _) = client.call(BEGIN, vec![dict([])]).await;
    assert_eq!(tag, SUCCESS);
    client.run("MATCH (n:Tx) RETURN count(n)", dict([])).await;
    let (tag, _) = client.call(ROLLBACK, vec![]).await;
    assert_eq!(tag, SUCCESS);

    // Each write is atomic and durable on its own, so a later ROLLBACK cannot
    // undo it: the server says so with a stable code instead of pretending.
    let (tag, _) = client.call(BEGIN, vec![dict([])]).await;
    assert_eq!(tag, SUCCESS);
    client.run("CREATE (n:Tx {k: 3})", dict([])).await;
    let (tag, fields) = client.call(ROLLBACK, vec![]).await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Transaction.RollbackNotSupported"
    );
    client.call(RESET, vec![]).await;
    let (_, records, _) = client.run("MATCH (n:Tx) RETURN count(n)", dict([])).await;
    assert_eq!(records, vec![vec![Value::Integer(3)]]);

    // Transaction control outside a transaction is a request error.
    let (tag, fields) = client.call(COMMIT, vec![]).await;
    assert_eq!(tag, FAILURE);
    assert_eq!(failure_code(&fields), "Neo.ClientError.Request.Invalid");
    server.abort();
}

#[tokio::test]
async fn parameters_round_trip_with_their_types_and_unsupported_ones_fail_cleanly() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::ready(port).await;

    let params = dict([
        ("name", str("north")),
        ("n", Value::Integer(7)),
        ("f", Value::Float(1.5)),
        ("b", Value::Boolean(false)),
        ("l", Value::List(vec![Value::Integer(1), Value::Integer(2)])),
    ]);
    let (_, records, _) = client
        .run(
            "CREATE (p:Param {name: $name, n: $n, f: $f, b: $b, l: $l}) RETURN p.name, p.n, p.f, p.b, p.l",
            params,
        )
        .await;
    assert_eq!(
        records,
        vec![vec![
            str("north"),
            Value::Integer(7),
            Value::Float(1.5),
            Value::Boolean(false),
            Value::List(vec![Value::Integer(1), Value::Integer(2)]),
        ]]
    );

    let (tag, fields) = client
        .call(
            RUN,
            vec![
                str("MATCH (p:Param) WHERE p.name = $blob RETURN p.name LIMIT 1"),
                dict([("blob", Value::Bytes(vec![1, 2, 3]))]),
                dict([]),
            ],
        )
        .await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.ClientError.Statement.ArgumentError"
    );
    server.abort();
}

#[tokio::test]
async fn route_returns_the_single_server_for_every_role() {
    let (port, server) = start(state(&[])).await;
    let mut client = Client::ready(port).await;
    let address = format!("127.0.0.1:{port}");
    let (tag, fields) = client
        .call(
            ROUTE,
            vec![
                dict([("address", str(&address))]),
                Value::List(vec![]),
                dict([]),
            ],
        )
        .await;
    assert_eq!(tag, SUCCESS, "{fields:?}");
    let table = get(&fields[0], "rt").expect("ROUTE returns a routing table");
    assert_eq!(get(table, "db"), Some(&str("corrobore")));
    assert!(matches!(get(table, "ttl"), Some(Value::Integer(ttl)) if *ttl > 0));
    let Some(Value::List(servers)) = get(table, "servers") else {
        panic!("servers list expected");
    };
    let mut roles = servers
        .iter()
        .map(|server| {
            assert_eq!(
                get(server, "addresses"),
                Some(&Value::List(vec![str(&address)])),
                "every role points back at the address the client used"
            );
            text(get(server, "role").unwrap()).to_owned()
        })
        .collect::<Vec<_>>();
    roles.sort();
    assert_eq!(roles, vec!["READ", "ROUTE", "WRITE"]);
    server.abort();
}

#[tokio::test]
async fn goodbye_and_protocol_misuse_close_the_connection() {
    let (port, server) = start(state(&[])).await;

    let mut client = Client::ready(port).await;
    client.send(GOODBYE, vec![]).await;
    assert!(
        client.try_recv().await.is_none(),
        "GOODBYE has no reply and closes"
    );

    // RUN before HELLO is a protocol violation.
    let (mut client, _) = Client::connect(port, [version(5, 4, 4), 0, 0, 0]).await;
    let (tag, fields) = client.call(RUN, vec![str(PING), dict([]), dict([])]).await;
    assert_eq!(tag, FAILURE);
    assert_eq!(failure_code(&fields), "Neo.ClientError.Request.Invalid");
    assert!(client.try_recv().await.is_none());

    // An unknown signature is refused too.
    let mut client = Client::ready(port).await;
    let (tag, fields) = client.call(0x42, vec![]).await;
    assert_eq!(tag, FAILURE);
    assert_eq!(failure_code(&fields), "Neo.ClientError.Request.Invalid");
    server.abort();
}

#[tokio::test]
async fn listener_stops_accepting_once_the_lifecycle_drains() {
    let state = state(&[]);
    let lifecycle = Arc::clone(&state.lifecycle);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(serve_bolt(listener, state, None));

    let mut client = Client::ready(port).await;
    lifecycle.begin_draining();
    let finished = timeout(Duration::from_secs(5), server).await;
    assert!(
        matches!(finished, Ok(Ok(Ok(())))),
        "the listener returns cleanly when draining: {finished:?}"
    );
    // The open connection is told to go away rather than hanging.
    let (tag, fields) = client.call(RUN, vec![str(PING), dict([]), dict([])]).await;
    assert_eq!(tag, FAILURE);
    assert_eq!(
        failure_code(&fields),
        "Neo.TransientError.General.DatabaseUnavailable"
    );
}

#[test]
fn server_config_parses_bolt_settings_with_safe_defaults() {
    let config = ServerConfig::from_map(&HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        TOKEN.to_owned(),
    )]))
    .unwrap();
    assert_eq!(config.bolt_port, 7687);
    assert_eq!(config.bolt_max_connections, 64);

    let config = ServerConfig::from_map(&HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), TOKEN.to_owned()),
        ("CORROBORE_BOLT_PORT".to_owned(), "17687".to_owned()),
        ("CORROBORE_BOLT_MAX_CONNECTIONS".to_owned(), "8".to_owned()),
    ]))
    .unwrap();
    assert_eq!(config.bolt_port, 17687);
    assert_eq!(config.bolt_max_connections, 8);

    for (name, value) in [
        ("CORROBORE_BOLT_PORT", "eighty"),
        ("CORROBORE_BOLT_MAX_CONNECTIONS", "0"),
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
    let path = std::env::temp_dir().join(format!("corrobore-bolt-{name}-{suffix}"));
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
    bolt_port: u16,
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
            "--bolt-port",
            &bolt_port.to_string(),
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
fn standalone_start_opens_the_bolt_listener_only_when_the_interface_is_enabled() {
    let directory = temp_dir("standalone");
    let http_port = reserve_port();
    let bolt_port = reserve_port();
    let mut child = spawn_standalone(&directory, "http,bolt", http_port, bolt_port);
    assert!(wait_for(http_port), "HTTP listener should open");
    assert!(wait_for(bolt_port), "Bolt listener should open beside HTTP");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut client = Client::ready(bolt_port).await;
        let (_, records, _) = client.run(PING, dict([])).await;
        assert_eq!(records, vec![vec![Value::Integer(0)]]);
    });
    child.kill().unwrap();
    child.wait().unwrap();

    let directory = temp_dir("standalone-http-only");
    let http_port = reserve_port();
    let bolt_port = reserve_port();
    let mut child = spawn_standalone(&directory, "http", http_port, bolt_port);
    assert!(wait_for(http_port));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", bolt_port)).is_err(),
        "Bolt stays off unless the interface is enabled"
    );
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn validate_config_accepts_the_bolt_section_and_prints_it() {
    let directory = temp_dir("validate");
    let config_path = directory.join("corrobore.toml");
    fs::write(
        &config_path,
        format!(
            r#"
[server]
auth_token = "{TOKEN}"

[interfaces]
enabled = ["http", "bolt"]

[bolt]
port = 17690
max_connections = 12
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
        stdout.contains(r#"interfaces.enabled = ["http", "bolt"]"#),
        "{stdout}"
    );
    assert!(stdout.contains("bolt.port = 17690"), "{stdout}");
    assert!(stdout.contains("bolt.max_connections = 12"), "{stdout}");
}
