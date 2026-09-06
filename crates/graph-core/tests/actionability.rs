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
use graph_core::*;
fn c(v: f64) -> Confidence {
    Confidence::new(v).expect("confidence")
}
fn dimensions() -> ConfidenceDimensions {
    ConfidenceDimensions {
        evidence_sufficiency: Some(c(0.99)),
        source_independence: Some(c(0.9)),
        verifier_strength: Some(c(1.0)),
        contradiction_load: Some(c(0.0)),
        temporal_validity: Some(c(1.0)),
        ..Default::default()
    }
}
#[test]
fn fabricated_professional_evidence_cannot_replace_mechanical_verification() {
    let d = dimensions();
    let result = ActionabilityPolicy::default().evaluate(&d, 8, false, VerdictState::Supported);
    assert!(!result.is_actionable());
    assert_eq!(
        result.blockers(),
        &[ActionabilityBlocker::DeterministicVerificationMissing]
    );
    assert_eq!(d.display_confidence(), Some(c(0.9)));
}
#[test]
fn each_gate_blocks_alone_and_reasons_accumulate() {
    let policy = ActionabilityPolicy::default();
    assert!(
        policy
            .evaluate(&dimensions(), 2, true, VerdictState::Supported)
            .is_actionable()
    );
    assert_eq!(
        policy
            .evaluate(&dimensions(), 1, true, VerdictState::Supported)
            .blockers(),
        &[ActionabilityBlocker::IndependentCorroborationMissing]
    );
    let mut d = dimensions();
    d.contradiction_load = Some(c(0.8));
    assert_eq!(
        policy
            .evaluate(&d, 2, true, VerdictState::Supported)
            .blockers(),
        &[ActionabilityBlocker::ContradictionThresholdExceeded]
    );
    d = dimensions();
    d.temporal_validity = Some(c(0.0));
    assert_eq!(
        policy
            .evaluate(&d, 2, true, VerdictState::Supported)
            .blockers(),
        &[ActionabilityBlocker::TemporalValidityStale]
    );
    d.contradiction_load = Some(c(0.8));
    assert_eq!(
        policy
            .evaluate(&d, 1, false, VerdictState::Supported)
            .blockers()
            .len(),
        4
    );
    assert!(
        !policy
            .evaluate(&dimensions(), 2, true, VerdictState::Refuted)
            .is_actionable()
    );
}
#[test]
fn absent_inputs_abstain_and_display_never_fills_missing_dimensions() {
    let d = ConfidenceDimensions::default();
    let result = ActionabilityPolicy::default().evaluate(&d, 2, true, VerdictState::Supported);
    assert!(!result.is_actionable());
    assert!(result.dimension().is_none());
    assert!(d.display_confidence().is_none());
    let mut d = dimensions();
    d.verifier_strength = Some(c(0.5));
    assert_eq!(d.display_confidence(), Some(c(0.45)));
    d.verifier_strength = None;
    assert_eq!(d.display_confidence(), Some(c(0.9)));
}
#[test]
fn claim_type_policy_can_require_only_one_cluster_without_relaxing_other_gates() {
    let policy = ActionabilityPolicy::new("single-source-v1", 1, c(0.25)).expect("policy");
    assert!(
        policy
            .evaluate(&dimensions(), 1, true, VerdictState::Supported)
            .is_actionable()
    );
    assert!(
        !policy
            .evaluate(&dimensions(), 1, false, VerdictState::Supported)
            .is_actionable()
    );
}
#[test]
fn policy_paths_never_read_legacy_record_confidence() {
    for source in [
        include_str!("../src/verdict.rs"),
        include_str!("../src/cluster_aggregation.rs"),
        include_str!("../src/export_plan.rs"),
        include_str!("../src/semantic_seed_graph_resolver.rs"),
    ] {
        assert!(
            !source
                .split("#[cfg(test)]")
                .next()
                .expect("production source")
                .contains(".confidence()")
        );
    }
}
#[test]
fn blocked_claim_cannot_export_even_when_status_and_scalar_look_ready() {
    let mut graph = Graph::new();
    let node = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_status(RecordStatus::Exportable)
                .with_confidence(c(1.0)),
        )
        .expect("node");
    let claim = ClaimId::new("claim--fabricated").expect("id");
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("professional-looking assertion").expect("statement"),
            ClaimTarget::Node(node),
        ))
        .expect("claim");
    let evidence = EvidenceRecordStore::new();
    let stores = graph.epistemic_stores_mut();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    );
    let t = TemporalTimestamp::new("2026-09-06T00:00:00Z").expect("time");
    resolve_current_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &claim,
        BitemporalStamp::new(t.clone(), t).expect("stamp"),
    )
    .expect("resolve");
    for mode in [ExportMode::Strict, ExportMode::Permissive] {
        let metadata = ExportMetadata::new(
            "snapshot--test",
            TransactionId::new("transaction--test").expect("id"),
            "v1",
            ExportProfile::FimiJsonMvp,
            mode,
            None,
        )
        .expect("metadata");
        let result = build_deterministic_export_plan_with_options(
            &graph,
            metadata,
            &[],
            ExportPlanOptions::default().with_force_validation(true),
        );
        if mode == ExportMode::Strict {
            assert!(
                result
                    .expect_err("blocked")
                    .to_string()
                    .contains("deterministic_verification_missing")
            );
        } else {
            let plan = result.expect("permissive");
            assert!(plan.records().is_empty());
            assert!(
                plan.warnings()
                    .iter()
                    .any(|w| w.message().contains("deterministic_verification_missing"))
            );
        }
    }
}
