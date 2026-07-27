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

//! Parameter values keep their scalar type across the public engine boundary.
//!
//! Flattening parameters to text used to produce wrong row sets rather than
//! errors: a bound `LIMIT` matched nothing, and a number written through a
//! parameter never compared equal to a numeric literal. Silent wrong answers are
//! the failure mode these tests exist to prevent.

use std::collections::HashMap;

use corrobore_engine::{
    CorroboreEngine, CypherResponseData, CypherResponseStatus, CypherValue, EngineRequest,
    EngineRequestMode,
};

fn typed(pairs: &[(&str, CypherValue)]) -> HashMap<String, CypherValue> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn row_count(data: &CypherResponseData) -> usize {
    match data {
        CypherResponseData::Records(records) => records.len(),
        _ => 0,
    }
}

fn seed_indicators(engine: &mut CorroboreEngine, names: &[&str]) {
    for name in names {
        let request = EngineRequest::new(
            "CREATE (n:Indicator {name: $name})",
            EngineRequestMode::Mutation,
        )
        .with_typed_parameters(typed(&[("name", CypherValue::String((*name).to_owned()))]));
        engine.execute_request(request).expect("seed should apply");
    }
}

#[test]
fn bound_row_count_limits_rows_like_a_literal() {
    let mut engine = CorroboreEngine::strict_default();
    seed_indicators(&mut engine, &["a", "b", "c"]);

    let unbounded = engine.read("MATCH (n) RETURN n").expect("read all");
    assert_eq!(row_count(&unbounded.data), 3);

    let literal = engine
        .read("MATCH (n) RETURN n LIMIT 2")
        .expect("literal limit");
    assert_eq!(row_count(&literal.data), 2);

    let bound = EngineRequest::new(
        "MATCH (n) RETURN n LIMIT $limit",
        EngineRequestMode::ReadOnly,
    )
    .with_typed_parameters(typed(&[("limit", CypherValue::Integer(2))]));
    let bound = engine.execute_request(bound).expect("bound limit");

    assert_eq!(
        row_count(&bound.data),
        row_count(&literal.data),
        "a bound row count must behave exactly like the literal"
    );
}

#[test]
fn bound_skip_offsets_rows() {
    let mut engine = CorroboreEngine::strict_default();
    seed_indicators(&mut engine, &["a", "b", "c"]);

    let bound = EngineRequest::new(
        "MATCH (n) RETURN n SKIP $offset",
        EngineRequestMode::ReadOnly,
    )
    .with_typed_parameters(typed(&[("offset", CypherValue::Integer(2))]));
    let bound = engine.execute_request(bound).expect("bound skip");

    assert_eq!(row_count(&bound.data), 1);
}

#[test]
fn number_written_through_a_parameter_compares_equal_to_a_numeric_literal() {
    let mut engine = CorroboreEngine::strict_default();
    let create = EngineRequest::new(
        "CREATE (n:Metric {score: $score})",
        EngineRequestMode::Mutation,
    )
    .with_typed_parameters(typed(&[("score", CypherValue::Integer(42))]));
    engine.execute_request(create).expect("create metric");

    let literal = engine
        .read("MATCH (n:Metric) WHERE n.score = 42 RETURN n.score")
        .expect("literal comparison");
    assert_eq!(
        row_count(&literal.data),
        1,
        "a parameter-written number must match an integer literal"
    );

    let bound = EngineRequest::new(
        "MATCH (n:Metric) WHERE n.score = $score RETURN n.score",
        EngineRequestMode::ReadOnly,
    )
    .with_typed_parameters(typed(&[("score", CypherValue::Integer(42))]));
    let bound = engine.execute_request(bound).expect("bound comparison");
    assert_eq!(row_count(&bound.data), 1);
}

#[test]
fn boolean_parameter_keeps_its_type() {
    let mut engine = CorroboreEngine::strict_default();
    let create = EngineRequest::new(
        "CREATE (n:Flagged {active: $active})",
        EngineRequestMode::Mutation,
    )
    .with_typed_parameters(typed(&[("active", CypherValue::Boolean(true))]));
    engine.execute_request(create).expect("create flagged");

    let matched = engine
        .read("MATCH (n:Flagged) WHERE n.active = true RETURN n.active")
        .expect("boolean comparison");
    assert_eq!(row_count(&matched.data), 1);
}

#[test]
fn unbound_placeholder_fails_loudly_instead_of_returning_no_rows() {
    let mut engine = CorroboreEngine::strict_default();
    seed_indicators(&mut engine, &["a"]);

    let request = EngineRequest::new(
        "MATCH (n) RETURN n LIMIT $limit",
        EngineRequestMode::ReadOnly,
    );

    match engine.execute_request(request) {
        Err(_) => {}
        Ok(response) => assert_ne!(
            response.status,
            CypherResponseStatus::Success,
            "a missing binding must not be reported as a successful read"
        ),
    }
}

#[test]
fn row_count_bound_to_a_string_is_rejected() {
    let mut engine = CorroboreEngine::strict_default();
    seed_indicators(&mut engine, &["a"]);

    // The value looks numeric but is typed as text; the position requires an
    // integer, so this is a type error rather than an ignored bound.
    let request = EngineRequest::new(
        "MATCH (n) RETURN n LIMIT $limit",
        EngineRequestMode::ReadOnly,
    )
    .with_typed_parameters(typed(&[("limit", CypherValue::String("1".to_owned()))]));

    match engine.execute_request(request) {
        Err(_) => {}
        Ok(response) => assert_ne!(
            response.status,
            CypherResponseStatus::Success,
            "a string bound to LIMIT must not be accepted"
        ),
    }
}

#[test]
fn parameter_value_cannot_become_query_syntax() {
    let mut engine = CorroboreEngine::strict_default();
    seed_indicators(&mut engine, &["seed"]);

    for payload in [
        "seed' CREATE (m:Pwned) RETURN m",
        "seed\\' CREATE (m:Pwned) RETURN m",
        "seed' DELETE n RETURN n",
    ] {
        let request = EngineRequest::new(
            "MATCH (n) WHERE n.name = $x RETURN n",
            EngineRequestMode::ReadOnly,
        )
        .with_typed_parameters(typed(&[("x", CypherValue::String(payload.to_owned()))]));
        let _ = engine.execute_request(request);

        let pwned = engine
            .read("MATCH (p:Pwned) RETURN p")
            .expect("probe should read");
        assert_eq!(
            row_count(&pwned.data),
            0,
            "payload leaked syntax: {payload:?}"
        );

        let total = engine
            .read("MATCH (n) RETURN n")
            .expect("count should read");
        assert_eq!(row_count(&total.data), 1, "graph changed for {payload:?}");
    }
}

#[test]
fn quote_bearing_value_round_trips_without_escape_characters() {
    let mut engine = CorroboreEngine::strict_default();
    let create = EngineRequest::new(
        "CREATE (n:Person {name: $name})",
        EngineRequestMode::Mutation,
    )
    .with_typed_parameters(typed(&[(
        "name",
        CypherValue::String("O'Brien".to_owned()),
    )]));
    engine.execute_request(create).expect("create person");

    let stored = match engine
        .read("MATCH (n:Person) RETURN n.name")
        .expect("read person")
        .data
    {
        CypherResponseData::Records(records) => records
            .first()
            .and_then(|record| record.fields.values().next().cloned())
            .expect("name should be projected"),
        other => panic!("expected records, got {other:?}"),
    };

    assert_eq!(stored, "O'Brien");
}
