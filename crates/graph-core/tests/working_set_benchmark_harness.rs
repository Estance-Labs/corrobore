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
use graph_core::{
    BenchmarkPolicyKind, fimi_multi_hop_benchmark_workload, run_working_set_benchmark,
};

//
// Verify that the built-in workload is deterministic: the benchmark's inputs
// must be identical across runs for reports to be comparable.
//
// Given two independently generated workloads,
// when their declared inputs are compared,
// then the seeds, expected evidence edges, labels, and budgets should be equal.
#[test]
fn workload_generator_is_deterministic() {
    let first = fimi_multi_hop_benchmark_workload();
    let second = fimi_multi_hop_benchmark_workload();

    assert_eq!(first.name, second.name);
    assert_eq!(first.seed_node_ids, second.seed_node_ids);
    assert_eq!(
        first.expected_evidence_relationship_ids,
        second.expected_evidence_relationship_ids
    );
    assert_eq!(first.relevant_labels, second.relevant_labels);
    assert_eq!(first.task_label, second.task_label);
    assert_eq!(first.max_source_expansions, second.max_source_expansions);
    assert_eq!(first.expansion_budget, second.expansion_budget);
    assert!(
        first.expected_evidence_relationship_ids.len() >= 3,
        "the workload should require a multi-hop evidence chain"
    );
}

//
// Verify that one benchmark run compares all seven policies of the epic in a
// stable order, so reports are diffable across runs and machines.
//
// Given the built-in workload,
// when the benchmark runs,
// then the report should contain one metrics row per policy, in the declared
// policy order.
#[test]
fn report_covers_all_seven_policies_in_stable_order() {
    let workload = fimi_multi_hop_benchmark_workload();

    let report = run_working_set_benchmark(&workload).expect("benchmark should run");

    assert_eq!(report.workload_name, workload.name);
    assert_eq!(report.policy_metrics.len(), BenchmarkPolicyKind::ALL.len());
    let reported: Vec<BenchmarkPolicyKind> = report
        .policy_metrics
        .iter()
        .map(|metrics| metrics.policy)
        .collect();
    assert_eq!(reported, BenchmarkPolicyKind::ALL.to_vec());
}

//
// Verify that every policy row carries the epic's metrics within their valid
// bounds: pages loaded, peak resident records, p95 step cost, and recall.
//
// Given a completed benchmark run,
// when each policy row is inspected,
// then pages and peak counters should be positive and recall should lie in
// [0, 1].
#[test]
fn metrics_are_populated_within_bounds() {
    let workload = fimi_multi_hop_benchmark_workload();

    let report = run_working_set_benchmark(&workload).expect("benchmark should run");

    for metrics in &report.policy_metrics {
        assert!(
            metrics.pages_loaded > 0,
            "{:?} should page in at least the seed",
            metrics.policy
        );
        assert!(
            metrics.peak_resident_records > 0,
            "{:?} should hold resident records",
            metrics.policy
        );
        assert!(
            (0.0..=1.0).contains(&metrics.evidence_recall),
            "{:?} recall should be a ratio",
            metrics.policy
        );
        assert!(
            metrics.p95_step_page_in_count <= metrics.pages_loaded,
            "{:?} p95 step cost cannot exceed the total",
            metrics.policy
        );
    }
}

//
// Verify full reproducibility: two runs over the same workload must produce
// exactly equal reports, which the epic's acceptance criteria require.
//
// Given the built-in workload,
// when the benchmark runs twice,
// then both reports should be identical.
#[test]
fn two_runs_produce_identical_reports() {
    let workload = fimi_multi_hop_benchmark_workload();

    let first = run_working_set_benchmark(&workload).expect("first run should succeed");
    let second = run_working_set_benchmark(&workload).expect("second run should succeed");

    assert_eq!(first, second);
}

//
// Verify that the budget lever works: with a generous source-expansion budget,
// every policy exhausts the reachable graph and reaches full evidence recall.
//
// Given the workload with a raised expansion allowance,
// when the benchmark runs,
// then every policy should reach recall 1.0.
#[test]
fn all_policies_reach_full_recall_with_a_generous_budget() {
    let mut workload = fimi_multi_hop_benchmark_workload();
    workload.max_source_expansions = 32;

    let report = run_working_set_benchmark(&workload).expect("benchmark should run");

    for metrics in &report.policy_metrics {
        assert_eq!(
            metrics.evidence_recall, 1.0,
            "{:?} should find the full evidence chain without budget pressure",
            metrics.policy
        );
    }
}

//
// Verify that the tight default budget separates informed policies from blind
// FIFO exploration on a workload whose distractors are discovered first: this
// is the comparison the epic's learned-policy targets are measured against.
//
// Given the default tight workload,
// when the benchmark runs,
// then FIFO (LRU) should miss part of the evidence chain while the
// semantic-only and learned pheromone policies should recover all of it.
#[test]
fn informed_policies_beat_fifo_under_a_tight_budget() {
    let workload = fimi_multi_hop_benchmark_workload();

    let report = run_working_set_benchmark(&workload).expect("benchmark should run");

    let recall_of = |policy: BenchmarkPolicyKind| {
        report
            .policy_metrics
            .iter()
            .find(|metrics| metrics.policy == policy)
            .unwrap_or_else(|| panic!("{policy:?} should be reported"))
            .evidence_recall
    };

    assert!(
        recall_of(BenchmarkPolicyKind::Lru) < 1.0,
        "FIFO should waste its tight budget on the distractors discovered first"
    );
    assert_eq!(
        recall_of(BenchmarkPolicyKind::SemanticOnly),
        1.0,
        "label-guided selection should follow the relevant chain"
    );
    assert_eq!(
        recall_of(BenchmarkPolicyKind::LearnedPheromone),
        1.0,
        "the pheromone policy should avoid the dead ends learned in its warm-up episode"
    );
}
