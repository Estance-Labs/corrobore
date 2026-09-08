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
use super::*;
use serde_json::json;

fn corpus() -> Value {
    json!([
        {"id": "ev-operator", "entity": "registry", "stance": "supports",
         "text": "The concession registry records Aster as the operator of the north relay."},
        {"id": "ev-capacity", "entity": "registry", "stance": "refutes",
         "text": "The certification registry records a capacity of 300 for the south depot."},
        {"id": "ev-noise", "entity": "operations-desk", "stance": "neutral",
         "text": "Unrelated maintenance bulletin about turbine lubrication schedules."}
    ])
}

fn retrieve_request(text: &str) -> Value {
    json!({
        "schemaVersion": "corrobore-evidence-suite-request-v1",
        "asOf": "2026-06-01T00:00:00Z",
        "operation": "retrieve",
        "claim": {"id": "claim-1", "text": text},
        "corpus": corpus()
    })
}

fn evaluate_request(evidence: Value) -> Value {
    json!({
        "schemaVersion": "corrobore-evidence-suite-request-v1",
        "asOf": "2026-06-01T00:00:00Z",
        "operation": "evaluate",
        "claim": {"id": "claim-1", "text": "Aster operates the north relay."},
        "corpus": corpus(),
        "evidence": evidence
    })
}

#[test]
fn retrieval_ranks_the_matching_document_and_excludes_noise() {
    let result = evaluate(retrieve_request("Aster operator north relay")).expect("evaluate");

    let ids: Vec<&str> = result["evidenceIds"]
        .as_array()
        .expect("array")
        .iter()
        .map(|value| value.as_str().expect("string"))
        .collect();
    assert!(ids.contains(&"ev-operator"), "got {ids:?}");
    assert!(!ids.contains(&"ev-noise"), "got {ids:?}");
}

#[test]
fn retrieval_never_reads_the_evaluator_owned_claim_label() {
    // A corpus carrying claimId must not let retrieval short-circuit on it.
    let mut request = retrieve_request("turbine lubrication schedules");
    request["corpus"][0]["claimId"] = json!("claim-1");
    let result = evaluate(request).expect("evaluate");

    let ids: Vec<&str> = result["evidenceIds"]
        .as_array()
        .expect("array")
        .iter()
        .map(|value| value.as_str().expect("string"))
        .collect();
    assert_eq!(
        ids,
        vec!["ev-noise"],
        "retrieval must follow the objective text"
    );
}

#[test]
fn supporting_evidence_resolves_a_supported_verdict_across_stages() {
    let result = evaluate(evaluate_request(json!([corpus()[0]]))).expect("evaluate");
    let stages = &result["stages"];

    assert_eq!(result["engine"], "graph-core");
    assert_eq!(stages["extraction"], json!(["ev-operator"]));
    assert_eq!(stages["entity_resolution"], json!(["registry"]));
    assert_eq!(stages["subgraph_construction"], json!(["ev-operator"]));
    assert_eq!(stages["evidence_sufficiency"], json!(["sufficient"]));
    assert_eq!(stages["verdict"], json!(["supported"]));
}

#[test]
fn neutral_evidence_attaches_no_stance_and_stays_insufficient() {
    let result = evaluate(evaluate_request(json!([corpus()[2]]))).expect("evaluate");
    let stages = &result["stages"];

    assert_eq!(stages["extraction"], json!(["ev-noise"]));
    assert_eq!(stages["subgraph_construction"], json!([]));
    assert_eq!(stages["evidence_sufficiency"], json!(["insufficient"]));
}

#[test]
fn the_harness_synthesized_stage_is_never_reported() {
    let result = evaluate(evaluate_request(json!([corpus()[0]]))).expect("evaluate");

    assert!(result["stages"].get("retrieval").is_none());
}

#[test]
fn an_unsupported_operation_or_version_fails() {
    let mut version = retrieve_request("anything");
    version["schemaVersion"] = json!("corrobore-evidence-suite-request-v999");
    assert!(evaluate(version).is_err());

    let mut operation = retrieve_request("anything");
    operation["operation"] = json!("guess");
    assert!(evaluate(operation).is_err());
}

#[test]
fn retrieval_reports_the_items_it_returned() {
    let result = evaluate(retrieve_request("Aster operator north relay")).expect("evaluate");
    let retrieval = &result["instrumentation"]["retrieval"];

    assert_eq!(retrieval["inputs"], 1);
    assert_eq!(
        retrieval["outputs"],
        result["evidenceIds"].as_array().unwrap().len()
    );
}

#[test]
fn extraction_counters_follow_the_workload_rather_than_a_constant() {
    let one = evaluate(evaluate_request(json!([corpus()[0]]))).expect("evaluate");
    let two = evaluate(evaluate_request(json!([corpus()[0], corpus()[1]]))).expect("evaluate");

    assert_eq!(one["instrumentation"]["extraction"]["inputs"], 1);
    assert_eq!(two["instrumentation"]["extraction"]["inputs"], 2);
}

#[test]
fn a_document_that_legitimately_produced_no_assertion_is_not_a_failure() {
    // The neutral document is ingested and attaches no signal link, which the
    // shortfall between inputs and outputs already states. Reporting it as a
    // failure would let the gate treat a false failure rate as the allowance a
    // candidate may regress into.
    let result = evaluate(evaluate_request(json!([corpus()[0], corpus()[2]]))).expect("evaluate");
    let extraction = &result["instrumentation"]["extraction"];

    assert_eq!(extraction["inputs"], 2);
    assert_eq!(extraction["outputs"], 1);
    assert_eq!(extraction["failures"], 0);
}

#[test]
fn every_stage_reports_a_zero_failure_rate_when_nothing_failed() {
    let result = evaluate(evaluate_request(json!([corpus()[0], corpus()[2]]))).expect("evaluate");
    let instrumentation = result["instrumentation"]
        .as_object()
        .expect("instrumentation");

    assert!(!instrumentation.is_empty());
    for (stage, counters) in instrumentation {
        assert_eq!(
            counters["failures"], 0,
            "{stage} reported a phantom failure"
        );
    }
}

#[test]
fn evaluation_never_reports_the_retrieval_stage_counters() {
    let result = evaluate(evaluate_request(json!([corpus()[0]]))).expect("evaluate");

    assert!(result["instrumentation"].get("retrieval").is_none());
}
