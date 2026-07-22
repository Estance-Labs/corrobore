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
use shared_runtime::{
    BenchmarkCorpus, BenchmarkExpectation, BenchmarkFixture, BenchmarkWorkflowInput, RuntimeError,
};

fn valid_fixture(fixture_id: &str) -> BenchmarkFixture {
    BenchmarkFixture {
        fixture_id: fixture_id.to_owned(),
        description: "simple correction workflow".to_owned(),
        baseline: BenchmarkWorkflowInput {
            workflow_name: "flat-json-baseline".to_owned(),
            steps: vec![
                "load incident context".to_owned(),
                "apply correction".to_owned(),
            ],
        },
        graph: BenchmarkWorkflowInput {
            workflow_name: "graph-workflow".to_owned(),
            steps: vec![
                "query impacted entities".to_owned(),
                "apply targeted update".to_owned(),
            ],
        },
        expected: BenchmarkExpectation {
            requires_export_validation: true,
            requires_audit_coverage: true,
            requires_snapshot_reproducibility: true,
        },
    }
}

#[test]
fn benchmark_corpus_validation_accepts_valid_minimal_fixture_set() {
    let corpus = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![valid_fixture("fixture-001")],
    };

    corpus
        .validate()
        .expect("valid benchmark corpus should pass validation");
}

#[test]
fn benchmark_corpus_validation_rejects_empty_schema_version() {
    let corpus = BenchmarkCorpus {
        schema_version: " ".to_owned(),
        fixtures: vec![valid_fixture("fixture-001")],
    };

    let error = corpus
        .validate()
        .expect_err("empty schema version should be rejected");

    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkCorpus("schema_version")
    );
}

#[test]
fn benchmark_corpus_validation_rejects_duplicate_fixture_ids() {
    let corpus = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![valid_fixture("fixture-001"), valid_fixture("fixture-001")],
    };

    let error = corpus
        .validate()
        .expect_err("duplicate fixture ids should be rejected");

    assert_eq!(
        error,
        RuntimeError::DuplicateBenchmarkFixtureId("fixture-001".to_owned())
    );
}

#[test]
fn benchmark_corpus_validation_rejects_fixture_without_graph_steps() {
    let mut fixture = valid_fixture("fixture-001");
    fixture.graph.steps.clear();

    let corpus = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture],
    };

    let error = corpus
        .validate()
        .expect_err("graph workflow steps are required");

    assert_eq!(error, RuntimeError::InvalidBenchmarkCorpus("graph.steps"));
}

#[test]
fn benchmark_corpus_validation_rejects_empty_fixture_set() {
    let corpus = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![],
    };

    let error = corpus
        .validate()
        .expect_err("empty fixture set should be rejected");

    assert_eq!(error, RuntimeError::InvalidBenchmarkCorpus("fixtures"));
}

#[test]
fn benchmark_corpus_validation_rejects_blank_required_fixture_fields() {
    let mut fixture_without_description = valid_fixture("fixture-001");
    fixture_without_description.description = " ".to_owned();
    let error = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture_without_description],
    }
    .validate()
    .expect_err("blank description should be rejected");
    assert_eq!(error, RuntimeError::InvalidBenchmarkCorpus("description"));

    let mut fixture_without_baseline_workflow = valid_fixture("fixture-002");
    fixture_without_baseline_workflow.baseline.workflow_name = " ".to_owned();
    let error = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture_without_baseline_workflow],
    }
    .validate()
    .expect_err("blank baseline workflow name should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkCorpus("baseline.workflow_name")
    );

    let mut fixture_without_baseline_steps = valid_fixture("fixture-003");
    fixture_without_baseline_steps.baseline.steps.clear();
    let error = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture_without_baseline_steps],
    }
    .validate()
    .expect_err("empty baseline steps should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkCorpus("baseline.steps")
    );

    let mut fixture_without_graph_workflow = valid_fixture("fixture-004");
    fixture_without_graph_workflow.graph.workflow_name = " ".to_owned();
    let error = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture_without_graph_workflow],
    }
    .validate()
    .expect_err("blank graph workflow name should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkCorpus("graph.workflow_name")
    );

    let mut fixture_without_id = valid_fixture("fixture-005");
    fixture_without_id.fixture_id = " ".to_owned();
    let error = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture_without_id],
    }
    .validate()
    .expect_err("blank fixture id should be rejected");
    assert_eq!(error, RuntimeError::InvalidBenchmarkCorpus("fixture_id"));
}
