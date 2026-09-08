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

fn request(claim: Value, evidence: Vec<Value>) -> Value {
    json!({
        "schemaVersion": "corrobore-abstention-request-v1",
        "asOf": "2026-06-01T00:00:00Z",
        "authorityPolicy": {"version": "fixture-v1", "trustedSourceIds": ["registry"]},
        "input": {"claim": claim, "evidence": evidence}
    })
}

fn claim(value: &str) -> Value {
    json!({"subject": "north-relay", "predicate": "operator", "value": value})
}

fn document(id: &str, source: &str, subject: &str, predicate: &str, value: &str) -> Value {
    json!({
        "id": id,
        "sourceId": source,
        "body": "synthetic abstention fixture",
        "assertion": {"subject": subject, "predicate": predicate, "value": value},
        "validFrom": "2026-01-01T00:00:00Z",
        "validUntil": null
    })
}

#[test]
fn matching_trusted_evidence_answers_supported() {
    let result = evaluate(request(
        claim("aster"),
        vec![document(
            "doc-1",
            "registry",
            "north-relay",
            "operator",
            "aster",
        )],
    ))
    .expect("evaluate");

    assert_eq!(result["engine"], "graph-core");
    assert_eq!(result["decision"]["action"], "answer");
    assert_eq!(result["decision"]["verdict"], "supported");
}

#[test]
fn conflicting_trusted_evidence_answers_refuted() {
    let result = evaluate(request(
        claim("boreal"),
        vec![document(
            "doc-1",
            "registry",
            "north-relay",
            "operator",
            "aster",
        )],
    ))
    .expect("evaluate");

    assert_eq!(result["decision"]["action"], "answer");
    assert_eq!(result["decision"]["verdict"], "refuted");
}

#[test]
fn absent_evidence_abstains() {
    let result = evaluate(request(claim("aster"), vec![])).expect("evaluate");

    assert_eq!(result["decision"]["action"], "abstain");
    assert!(result["decision"].get("verdict").is_none());
}

#[test]
fn evidence_about_another_fact_attaches_no_signal_and_abstains() {
    for (subject, predicate) in [
        ("south-relay", "operator"),
        ("north-relay", "commissioned-year"),
    ] {
        let result = evaluate(request(
            claim("aster"),
            vec![document("doc-1", "registry", subject, predicate, "aster")],
        ))
        .expect("evaluate");

        assert_eq!(
            result["decision"]["action"], "abstain",
            "{subject}/{predicate} must not resolve the claim"
        );
    }
}

#[test]
fn untrusted_source_carries_no_authority_and_abstains() {
    let result = evaluate(request(
        claim("aster"),
        vec![document(
            "doc-1",
            "anonymous-mirror",
            "north-relay",
            "operator",
            "aster",
        )],
    ))
    .expect("evaluate");

    assert_eq!(result["decision"]["action"], "abstain");
}

#[test]
fn evidence_valid_only_after_the_evaluation_instant_abstains() {
    let mut later = document("doc-1", "registry", "north-relay", "operator", "aster");
    later["validFrom"] = json!("2027-01-01T00:00:00Z");
    let result = evaluate(request(claim("aster"), vec![later])).expect("evaluate");

    assert_eq!(result["decision"]["action"], "abstain");
}

#[test]
fn a_malformed_request_fails_instead_of_abstaining() {
    let mut input = request(claim("aster"), vec![]);
    input["schemaVersion"] = json!("corrobore-abstention-request-v999");

    assert!(evaluate(input).is_err());
}

#[test]
fn duplicate_evidence_ids_are_rejected() {
    let result = evaluate(request(
        claim("aster"),
        vec![
            document("doc-1", "registry", "north-relay", "operator", "aster"),
            document("doc-1", "registry", "north-relay", "operator", "aster"),
        ],
    ));

    assert!(result.is_err());
}
