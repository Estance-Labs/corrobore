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

//! Execution bounds stop work while it happens, not after it finished.
//!
//! The property that matters for mutations is that a rejected query changed
//! nothing: enforcing a bound after the fact would report failure for writes the
//! caller can no longer undo.

use cypher_executor::{
    CypherPipelineExecutor, ExecutionError, ExecutionLimits, ExecutionPolicy, ExecutionResultData,
};
use cypher_parser::ParameterBindings;
use graph_core::Graph;

fn writable_executor() -> CypherPipelineExecutor {
    CypherPipelineExecutor::with_graph(
        ExecutionPolicy {
            read_only_by_default: false,
        },
        Graph::new(),
    )
}

fn seed(executor: &mut CypherPipelineExecutor, count: usize) {
    for index in 0..count {
        executor
            .execute(&format!("CREATE (n:Indicator {{name: 'seed-{index}'}})"))
            .expect("seed should apply");
    }
}

fn node_count(executor: &mut CypherPipelineExecutor) -> usize {
    match executor
        .execute("MATCH (n) RETURN n")
        .expect("count should run")
        .data
    {
        ExecutionResultData::Records(records) => records.len(),
        _ => 0,
    }
}

fn limits() -> ExecutionLimits {
    ExecutionLimits::unbounded()
}

#[test]
fn scan_stops_at_the_loaded_record_ceiling() {
    let mut executor = writable_executor();
    seed(&mut executor, 10);

    let bounded = ExecutionLimits {
        max_loaded_records: 4,
        ..limits()
    };
    let error = executor
        .execute_with_limits("MATCH (n) RETURN n", &ParameterBindings::new(), bounded)
        .expect_err("scanning past the ceiling must stop");

    match error {
        ExecutionError::LimitExceeded {
            dimension, limit, ..
        } => {
            assert_eq!(dimension, "loaded_records");
            assert_eq!(limit, 4);
        }
        other => panic!("expected a limit error, got {other:?}"),
    }
}

#[test]
fn scan_ceiling_counts_matched_rows_not_the_whole_graph() {
    let mut executor = writable_executor();
    seed(&mut executor, 10);

    // Only one row matches, so a ceiling below the graph size is still fine.
    let bounded = ExecutionLimits {
        max_loaded_records: 2,
        ..limits()
    };
    let result = executor
        .execute_with_limits(
            "MATCH (n:Indicator {name: 'seed-3'}) RETURN n",
            &ParameterBindings::new(),
            bounded,
        )
        .expect("a selective match must stay under the ceiling");

    match result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 1),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn returned_record_ceiling_is_enforced() {
    let mut executor = writable_executor();
    seed(&mut executor, 6);

    let bounded = ExecutionLimits {
        max_returned_records: 3,
        ..limits()
    };
    let error = executor
        .execute_with_limits("MATCH (n) RETURN n", &ParameterBindings::new(), bounded)
        .expect_err("projecting past the ceiling must be rejected");

    assert!(matches!(
        error,
        ExecutionError::LimitExceeded {
            dimension: "returned_records",
            ..
        }
    ));
}

#[test]
fn a_limit_clause_keeps_a_query_under_the_returned_ceiling() {
    let mut executor = writable_executor();
    seed(&mut executor, 6);

    let bounded = ExecutionLimits {
        max_returned_records: 3,
        ..limits()
    };
    let result = executor
        .execute_with_limits(
            "MATCH (n) RETURN n LIMIT 2",
            &ParameterBindings::new(),
            bounded,
        )
        .expect("LIMIT should bring the projection under the ceiling");

    match result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 2),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn over_budget_mutation_is_rejected_without_writing_anything() {
    let mut executor = writable_executor();
    seed(&mut executor, 5);
    let before = node_count(&mut executor);

    // Five matched rows times one assignment exceeds a ceiling of three.
    let bounded = ExecutionLimits {
        max_mutation_count: 3,
        ..limits()
    };
    let error = executor
        .execute_with_limits(
            "MATCH (n) SET n.tagged = 'yes'",
            &ParameterBindings::new(),
            bounded,
        )
        .expect_err("an over-budget mutation must be rejected");

    assert!(matches!(
        error,
        ExecutionError::LimitExceeded {
            dimension: "mutation_count",
            ..
        }
    ));

    // The decisive property: nothing was written before the rejection.
    assert_eq!(node_count(&mut executor), before);
    match executor
        .execute("MATCH (n) WHERE n.tagged = 'yes' RETURN n")
        .expect("tag probe should run")
        .data
    {
        ExecutionResultData::Records(records) => assert!(
            records.is_empty(),
            "a rejected mutation must not leave partial writes"
        ),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn over_budget_delete_leaves_the_graph_intact() {
    let mut executor = writable_executor();
    seed(&mut executor, 5);
    let before = node_count(&mut executor);

    let bounded = ExecutionLimits {
        max_mutation_count: 2,
        ..limits()
    };
    let error = executor
        .execute_with_limits("MATCH (n) DELETE n", &ParameterBindings::new(), bounded)
        .expect_err("an over-budget delete must be rejected");

    assert!(matches!(
        error,
        ExecutionError::LimitExceeded {
            dimension: "mutation_count",
            ..
        }
    ));
    assert_eq!(
        node_count(&mut executor),
        before,
        "a rejected delete must not tombstone anything"
    );
}

#[test]
fn mutation_within_budget_still_applies() {
    let mut executor = writable_executor();
    seed(&mut executor, 3);

    let bounded = ExecutionLimits {
        max_mutation_count: 10,
        ..limits()
    };
    executor
        .execute_with_limits(
            "MATCH (n) SET n.tagged = 'yes'",
            &ParameterBindings::new(),
            bounded,
        )
        .expect("a mutation inside its budget must apply");

    match executor
        .execute("MATCH (n) WHERE n.tagged = 'yes' RETURN n")
        .expect("tag probe should run")
        .data
    {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 3),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn an_expired_deadline_stops_a_scan_in_progress() {
    let mut executor = writable_executor();
    // The deadline is sampled every 1024 rows, so the scan must be long enough to
    // reach a sampling point.
    seed(&mut executor, 2_500);

    let bounded = ExecutionLimits {
        max_execution_time_ms: 0,
        ..limits()
    };
    let error = executor
        .execute_with_limits("MATCH (n) RETURN n", &ParameterBindings::new(), bounded)
        .expect_err("an exhausted deadline must stop the scan");

    assert!(matches!(
        error,
        ExecutionError::LimitExceeded {
            dimension: "execution_time_ms",
            ..
        }
    ));
}

#[test]
fn a_generous_deadline_does_not_interrupt_a_long_scan() {
    let mut executor = writable_executor();
    seed(&mut executor, 2_500);

    let bounded = ExecutionLimits {
        max_execution_time_ms: 60_000,
        ..limits()
    };
    let result = executor
        .execute_with_limits("MATCH (n) RETURN n", &ParameterBindings::new(), bounded)
        .expect("a generous deadline must let the scan finish");

    match result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 2_500),
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn unbounded_limits_do_not_change_behavior() {
    let mut executor = writable_executor();
    seed(&mut executor, 4);

    let bounded = executor
        .execute_with_limits(
            "MATCH (n) RETURN n",
            &ParameterBindings::new(),
            ExecutionLimits::unbounded(),
        )
        .expect("unbounded limits should never trigger");
    let plain = executor.execute("MATCH (n) RETURN n").expect("plain read");

    assert_eq!(bounded.data, plain.data);
}
