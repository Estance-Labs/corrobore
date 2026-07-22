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
use graph_core::{BitemporalFactStore, BitemporalStamp, FactId, GraphError, TemporalTimestamp};

fn fact_id(value: &str) -> FactId {
    FactId::new(value).expect("bitemporal fact ID should be valid")
}

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("bitemporal timestamp should be valid")
}

fn stamp(valid_from: &str, transaction_time: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts(valid_from), ts(transaction_time))
        .expect("bitemporal stamp should be valid")
}

fn stamp_until(valid_from: &str, valid_to: &str, transaction_time: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts(valid_from), ts(transaction_time))
        .expect("bitemporal stamp should be valid")
        .with_valid_to(ts(valid_to))
        .expect("valid-to should follow valid-from")
}

//
// Verify that one fact state carries the epic's four validated timestamp
// dimensions: when it was true in the world, when the engine recorded it, when
// the source observed it, and when the source published it.
//
// Given a stamp with all four dimensions,
// when its fields are read,
// then each dimension should be present with its value.
#[test]
fn stamps_carry_the_four_temporal_dimensions() {
    let stamp = BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-02-01T00:00:00Z"))
        .expect("stamp should be valid")
        .with_valid_to(ts("2026-06-01T00:00:00Z"))
        .expect("valid-to should follow valid-from")
        .with_observation_time(ts("2026-01-10T00:00:00Z"))
        .with_publication_time(ts("2026-01-15T00:00:00Z"));

    assert_eq!(stamp.valid_from.as_str(), "2026-01-01T00:00:00Z");
    assert_eq!(
        stamp.valid_to.as_ref().map(TemporalTimestamp::as_str),
        Some("2026-06-01T00:00:00Z")
    );
    assert_eq!(stamp.transaction_time.as_str(), "2026-02-01T00:00:00Z");
    assert_eq!(
        stamp
            .observation_time
            .as_ref()
            .map(TemporalTimestamp::as_str),
        Some("2026-01-10T00:00:00Z")
    );
    assert_eq!(
        stamp
            .publication_time
            .as_ref()
            .map(TemporalTimestamp::as_str),
        Some("2026-01-15T00:00:00Z")
    );
}

//
// Verify that stamps enforce the canonical comparable form and interval order
// as typed errors: chronology must never depend on caller formatting.
//
// Given non-canonical timestamps or an inverted valid interval,
// when a stamp is built,
// then construction should fail with `GraphError::InvalidBitemporalStamp`.
#[test]
fn stamps_reject_non_canonical_forms_and_inverted_intervals() {
    // Offset form is valid RFC3339 but not canonically comparable.
    let offset = TemporalTimestamp::new("2026-01-01T00:00:00+02:00")
        .expect("offset timestamp is valid RFC3339");
    let error = BitemporalStamp::new(offset, ts("2026-02-01T00:00:00Z"))
        .expect_err("non-canonical valid-from should return a typed error");
    assert!(matches!(error, GraphError::InvalidBitemporalStamp(_)));

    let error = BitemporalStamp::new(ts("2026-06-01T00:00:00Z"), ts("2026-02-01T00:00:00Z"))
        .expect("stamp should be valid")
        .with_valid_to(ts("2026-01-01T00:00:00Z"))
        .expect_err("an inverted valid interval should return a typed error");
    assert!(matches!(error, GraphError::InvalidBitemporalStamp(_)));
}

//
// Verify that successive states of the same fact coexist as versions: history
// is append-only and every state stays retrievable.
//
// Given a fact asserted with two successive valid intervals,
// when its history is read,
// then both states should be present in transaction order.
#[test]
fn successive_states_coexist_as_versions() {
    let mut store = BitemporalFactStore::new();
    let infrastructure = fact_id("fact--campaign-infrastructure");

    store
        .assert_fact_state(
            infrastructure.clone(),
            "Campaign uses hosting provider A",
            stamp_until(
                "2026-01-01T00:00:00Z",
                "2026-03-01T00:00:00Z",
                "2026-01-05T00:00:00Z",
            ),
        )
        .expect("first state should be asserted");
    store
        .assert_fact_state(
            infrastructure.clone(),
            "Campaign uses hosting provider B",
            stamp("2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z"),
        )
        .expect("second state should be asserted");

    let history = store.fact_history(&infrastructure);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].statement, "Campaign uses hosting provider A");
    assert_eq!(history[1].statement, "Campaign uses hosting provider B");
}

//
// Verify that contradictory states coexist without collapsing into a canonical
// value: both remain visible to as-of queries over their shared valid time.
//
// Given two contradictory states with overlapping valid intervals,
// when the fact is queried at a covered valid time,
// then both states should be returned.
#[test]
fn contradictory_states_coexist_without_canonical_collapse() {
    let mut store = BitemporalFactStore::new();
    let attribution = fact_id("fact--campaign-attribution");

    store
        .assert_fact_state(
            attribution.clone(),
            "Actor A operates the campaign",
            stamp("2026-01-01T00:00:00Z", "2026-01-10T00:00:00Z"),
        )
        .expect("first claim state should be asserted");
    store
        .assert_fact_state(
            attribution.clone(),
            "Actor B operates the campaign",
            stamp("2026-01-01T00:00:00Z", "2026-01-20T00:00:00Z"),
        )
        .expect("contradictory state should be asserted");

    let states = store.states_as_of(&attribution, &ts("2026-02-01T00:00:00Z"), None);
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].statement, "Actor A operates the campaign");
    assert_eq!(states[1].statement, "Actor B operates the campaign");
}

//
// Verify that overwrite-based temporal updates are forbidden at the contract
// level: recording into or before the engine's own past is a typed error, not
// a convention.
//
// Given a fact with a recorded state,
// when a new state is asserted with a transaction time not after the latest,
// then assertion should fail with `GraphError::BitemporalOverwriteForbidden`.
#[test]
fn overwrite_based_updates_are_a_typed_error() {
    let mut store = BitemporalFactStore::new();
    let fact = fact_id("fact--overwrite-guard");

    store
        .assert_fact_state(
            fact.clone(),
            "Original state",
            stamp("2026-01-01T00:00:00Z", "2026-01-10T00:00:00Z"),
        )
        .expect("original state should be asserted");

    for stale_transaction in ["2026-01-10T00:00:00Z", "2026-01-09T00:00:00Z"] {
        let error = store
            .assert_fact_state(
                fact.clone(),
                "Rewritten state",
                stamp("2026-01-01T00:00:00Z", stale_transaction),
            )
            .expect_err("recording into the engine's past should fail");
        assert!(matches!(
            error,
            GraphError::BitemporalOverwriteForbidden(id) if id == fact
        ));
    }

    assert_eq!(store.fact_history(&fact).len(), 1);
}

//
// Verify valid-time as-of semantics: a query at a valid time returns exactly
// the states whose valid interval covers it.
//
// Given a fact whose first state ends where the second begins,
// when the fact is queried at times inside each interval and before both,
// then each query should return the matching states only.
#[test]
fn valid_time_queries_select_covering_states() {
    let mut store = BitemporalFactStore::new();
    let infrastructure = fact_id("fact--valid-time");

    store
        .assert_fact_state(
            infrastructure.clone(),
            "Provider A",
            stamp_until(
                "2026-01-01T00:00:00Z",
                "2026-03-01T00:00:00Z",
                "2026-01-05T00:00:00Z",
            ),
        )
        .expect("first state should be asserted");
    store
        .assert_fact_state(
            infrastructure.clone(),
            "Provider B",
            stamp("2026-03-01T00:00:00Z", "2026-03-02T00:00:00Z"),
        )
        .expect("second state should be asserted");

    let during_first = store.states_as_of(&infrastructure, &ts("2026-02-01T00:00:00Z"), None);
    assert_eq!(during_first.len(), 1);
    assert_eq!(during_first[0].statement, "Provider A");

    let during_second = store.states_as_of(&infrastructure, &ts("2026-04-01T00:00:00Z"), None);
    assert_eq!(during_second.len(), 1);
    assert_eq!(during_second[0].statement, "Provider B");

    let before_both = store.states_as_of(&infrastructure, &ts("2025-12-01T00:00:00Z"), None);
    assert!(before_both.is_empty());
}

//
// Verify transaction-time as-of semantics: a query bounded by a transaction
// time reflects exactly what the engine knew then, without later recordings.
//
// Given a fact whose second state was recorded later,
// when the fact is queried as of a transaction time between the recordings,
// then only the first state should be visible, and an unbounded query should
// see both.
#[test]
fn transaction_time_queries_reflect_what_the_engine_knew() {
    let mut store = BitemporalFactStore::new();
    let attribution = fact_id("fact--transaction-time");

    store
        .assert_fact_state(
            attribution.clone(),
            "Actor A operates the campaign",
            stamp("2026-01-01T00:00:00Z", "2026-01-10T00:00:00Z"),
        )
        .expect("first state should be asserted");
    store
        .assert_fact_state(
            attribution.clone(),
            "Actor B operates the campaign",
            stamp("2026-01-01T00:00:00Z", "2026-02-10T00:00:00Z"),
        )
        .expect("second state should be asserted");

    let known_early = store.states_as_of(
        &attribution,
        &ts("2026-03-01T00:00:00Z"),
        Some(&ts("2026-01-15T00:00:00Z")),
    );
    assert_eq!(known_early.len(), 1);
    assert_eq!(known_early[0].statement, "Actor A operates the campaign");

    let known_now = store.states_as_of(&attribution, &ts("2026-03-01T00:00:00Z"), None);
    assert_eq!(known_now.len(), 2);
}

//
// Verify that unknown facts read as deterministically empty: absence is a
// stable answer, not an error, for both history and as-of queries.
//
// Given an empty store,
// when an unknown fact is queried,
// then history and as-of results should be empty.
#[test]
fn unknown_facts_read_as_deterministically_empty() {
    let store = BitemporalFactStore::new();
    let unknown = fact_id("fact--unknown");

    assert!(store.fact_history(&unknown).is_empty());
    assert!(
        store
            .states_as_of(&unknown, &ts("2026-01-01T00:00:00Z"), None)
            .is_empty()
    );
}

//
// Verify reproducibility: identical assertion sequences produce identical
// stores, so bitemporal history can be compared and replayed.
//
// Given two stores built by the same assertions,
// when they are compared,
// then they should be exactly equal.
#[test]
fn identical_assertion_sequences_produce_identical_stores() {
    let build = || {
        let mut store = BitemporalFactStore::new();
        store
            .assert_fact_state(
                fact_id("fact--reproducible"),
                "State",
                stamp("2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"),
            )
            .expect("state should be asserted");
        store
    };

    assert_eq!(build(), build());
}
