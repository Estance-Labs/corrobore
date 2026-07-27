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

//! Parameter binding must never let a value become Cypher syntax.
//!
//! Mode classification runs on the unbound query text, so a parameter value is
//! the one place where caller data reaches the parser. These tests pin the
//! read-only boundary against values that try to close their own string literal.

use std::collections::HashMap;

use graph_core::{SessionId, WorkspaceId};
use shared_runtime::{
    CypherBudgetRef, CypherGateway, CypherParameters, CypherRequest, CypherResponseData,
};

fn workspace_id() -> WorkspaceId {
    WorkspaceId::new("workspace--binding-safety").expect("workspace id should be valid")
}

fn session_id() -> SessionId {
    SessionId::new("session--binding-safety").expect("session id should be valid")
}

fn budget_ref() -> CypherBudgetRef {
    CypherBudgetRef::new("budget--binding-safety").expect("budget ref should be valid")
}

fn parameters(pairs: &[(&str, &str)]) -> CypherParameters {
    CypherParameters::new(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
    )
}

fn node_count(gateway: &mut CypherGateway) -> usize {
    let request = CypherRequest::build_read_only_request(
        "MATCH (n) RETURN n",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("count request should be valid");
    match gateway
        .execute(&request)
        .expect("count request should execute")
        .data
    {
        CypherResponseData::Records(records) => records.len(),
        _ => 0,
    }
}

fn seed(gateway: &mut CypherGateway) {
    let request = CypherRequest::build_mutation_request(
        "CREATE (n:Indicator {name: 'seed'})",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("seed request should be valid");
    gateway.execute(&request).expect("seed should execute");
}

#[test]
fn read_only_request_cannot_mutate_through_a_parameter_value() {
    // Each payload tries to terminate the bound literal and append a clause.
    let payloads = [
        "seed' CREATE (m:Pwned) RETURN m",
        "seed\\' CREATE (m:Pwned) RETURN m",
        "seed' CREATE (m:Pwned {name: 'owned'}) RETURN m",
        "seed' DELETE n RETURN n",
        "seed' SET n.owned = 'yes",
        "seed' RETURN n //",
    ];

    for payload in payloads {
        let mut gateway = CypherGateway::strict_default();
        seed(&mut gateway);
        let before = node_count(&mut gateway);

        let request = CypherRequest::build_read_only_request(
            "MATCH (n) WHERE n.name = $x RETURN n",
            parameters(&[("x", payload)]),
            workspace_id(),
            session_id(),
            budget_ref(),
        )
        .expect("read-only request should be valid");

        // The request may succeed (value simply matches nothing) or be rejected,
        // but it must never change the graph.
        let _ = gateway.execute(&request);

        assert_eq!(
            node_count(&mut gateway),
            before,
            "payload must not create or remove nodes: {payload:?}"
        );

        let pwned = CypherRequest::build_read_only_request(
            "MATCH (p:Pwned) RETURN p",
            CypherParameters::default(),
            workspace_id(),
            session_id(),
            budget_ref(),
        )
        .expect("probe request should be valid");
        let found = match gateway.execute(&pwned).expect("probe should execute").data {
            CypherResponseData::Records(records) => records.len(),
            _ => 0,
        };
        assert_eq!(
            found, 0,
            "payload must not create a Pwned node: {payload:?}"
        );
    }
}

#[test]
fn parameter_value_containing_a_quote_round_trips_without_the_escape_character() {
    let mut gateway = CypherGateway::strict_default();

    let create = CypherRequest::build_mutation_request(
        "CREATE (n:Person {name: $name}) RETURN n",
        parameters(&[("name", "O'Brien")]),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("mutation request should be valid");
    gateway
        .execute(&create)
        .expect("apostrophe value should be stored");

    let read = CypherRequest::build_read_only_request(
        "MATCH (n:Person) RETURN n.name",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("read request should be valid");

    let stored = match gateway.execute(&read).expect("read should execute").data {
        CypherResponseData::Records(records) => records
            .first()
            .and_then(|record| record.fields.values().next().cloned())
            .expect("stored value should be projected"),
        other => panic!("expected records, got {other:?}"),
    };

    // The escape character must stay out of the stored data.
    assert_eq!(stored, "O'Brien");
}
