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
//! Human judgments are attributable, append-only records beside machine evidence.
use graph_core::*;
use serde_json::json;
fn fixture() -> (Graph, ClaimId) {
    let mut graph = Graph::new();
    let claim = ClaimId::new("human-review").expect("id");
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("Machine claim").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
        ))
        .expect("claim");
    let stores = graph.epistemic_stores_mut();
    let source = SourceId::new("source").expect("source");
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://example.org/report",
            EvidenceSourceType::Document,
        ))
        .expect("source");
    let obs = ObservationId::new("observation").expect("observation");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                obs.clone(),
                source,
                "Original words",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");
    stores.claims.register_observation(obs.clone());
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(obs.clone()),
            claim.clone(),
            ClaimLinkKind::Supports,
        ))
        .expect("link");
    let stamp = BitemporalStamp::new(
        TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time"),
        TemporalTimestamp::new("2026-09-06T12:00:01Z").expect("time"),
    )
    .expect("stamp");
    stores
        .verifications
        .append(VerificationRecord::new(
            VerificationRecordId::new("check").expect("id"),
            "mechanical",
            "v1",
            true,
            VerificationInputs::for_claim(claim.clone()).with_observation(obs),
            VerificationResult::Pass,
            stamp.clone(),
        ))
        .expect("check");
    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    );
    resolve_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &claim,
        stamp,
        "ws-a-minimal-v1",
    )
    .expect("seed machine verdict");
    (graph, claim)
}
fn decision(id: &str, claim: &ClaimId, action: AnalystDecisionAction) -> AnalystDecision {
    AnalystDecision::new(
        id,
        claim.clone(),
        ActorId::new("analyst").expect("actor"),
        TemporalTimestamp::new("2026-09-06T18:00:00Z").expect("time"),
        action,
    )
    .expect("decision")
}
fn override_action() -> AnalystDecisionAction {
    AnalystDecisionAction::Override {
        judgment: "Needs further investigation".into(),
        rationale: "Independent analyst assessment".into(),
    }
}
#[test]
fn override_and_reversal_preserve_machine_records_and_remain_in_audit_after_restart() {
    let (mut graph, claim) = fixture();
    assert!(
        graph
            .epistemic_stores()
            .verdicts
            .current_verdict(&claim)
            .is_some()
    );
    let machine_before = serde_json::to_value(graph.epistemic_stores()).expect("machine");
    let original = decision("override-1", &claim, override_action());
    graph
        .record_analyst_decision(original.clone())
        .expect("override");
    graph
        .record_analyst_decision(original)
        .expect("idempotent retry");
    graph
        .record_analyst_decision(decision(
            "reverse-1",
            &claim,
            AnalystDecisionAction::Reversal {
                decision_id: "override-1".into(),
                rationale: "Withdrawn after review".into(),
            },
        ))
        .expect("reverse");
    let audit = graph.claim_audit_path(&claim).expect("audit");
    assert_eq!(
        audit["analyst_decisions"]
            .as_array()
            .expect("decisions")
            .len(),
        2
    );
    assert_eq!(audit["analyst_decisions"][0]["action"]["kind"], "override");
    assert_eq!(audit["analyst_decisions"][1]["action"]["kind"], "reversal");
    let mut machine_after = serde_json::to_value(graph.epistemic_stores()).expect("machine");
    machine_after
        .as_object_mut()
        .expect("stores")
        .remove("analyst_decisions");
    assert_eq!(machine_before, machine_after);
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restart");
    assert_eq!(audit, restored.claim_audit_path(&claim).expect("audit"));
}
#[test]
fn attribution_and_reversal_targets_are_required_and_failures_are_atomic() {
    let (mut graph, claim) = fixture();
    assert!(
        AnalystDecision::new(
            "",
            claim.clone(),
            ActorId::new("analyst").expect("actor"),
            TemporalTimestamp::new("2026-09-06T18:00:00Z").expect("time"),
            override_action()
        )
        .is_err()
    );
    let valid = decision(
        "annotation",
        &claim,
        AnalystDecisionAction::Annotation {
            text: "Check original source".into(),
        },
    );
    let mut forged = serde_json::to_value(&valid).expect("value");
    forged["actor"] = json!({"value":""});
    if let Ok(invalid) = serde_json::from_value(forged) {
        assert!(graph.record_analyst_decision(invalid).is_err());
    }
    graph.record_analyst_decision(valid).expect("annotation");
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    assert!(
        graph
            .record_analyst_decision(decision(
                "missing",
                &claim,
                AnalystDecisionAction::Reversal {
                    decision_id: "absent".into(),
                    rationale: "reason".into()
                }
            ))
            .is_err()
    );
    assert!(
        graph
            .record_analyst_decision(decision("annotation", &claim, override_action()))
            .is_err()
    );
    assert_eq!(
        before,
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot")
    );
}

#[test]
fn cross_claim_double_and_recursive_reversals_are_rejected() {
    let (mut graph, claim) = fixture();
    let other = ClaimId::new("other").expect("id");
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            other.clone(),
            ClaimStatement::new("Other claim").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("other", None)),
        ))
        .expect("claim");
    graph
        .record_analyst_decision(decision("original", &claim, override_action()))
        .expect("record");
    let reverse = |id: &str, claim: &ClaimId, target: &str| {
        decision(
            id,
            claim,
            AnalystDecisionAction::Reversal {
                decision_id: target.into(),
                rationale: "Withdrawn".into(),
            },
        )
    };
    assert!(
        graph
            .record_analyst_decision(reverse("cross", &other, "original"))
            .is_err()
    );
    graph
        .record_analyst_decision(reverse("reverse", &claim, "original"))
        .expect("reverse");
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    assert!(
        graph
            .record_analyst_decision(reverse("twice", &claim, "original"))
            .is_err()
    );
    assert!(
        graph
            .record_analyst_decision(reverse("recursive", &claim, "reverse"))
            .is_err()
    );
    assert_eq!(
        before,
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot")
    );
    let mut forged = before;
    forged["epistemic"]["analyst_decisions"]["records"][1]["action"]["decision_id"] =
        json!("missing");
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(forged).expect("decode")).is_err()
    );
}
