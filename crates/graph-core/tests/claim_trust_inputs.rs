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
    ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement, ClaimStatus, ClaimStore,
    ClaimTarget, GraphError, TemporalMetadata, TrustInputInput, TrustInputKind,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn create_asserted_claim(store: &mut ClaimStore, id: &ClaimId, statement: &str) {
    let input = ClaimInput::new(
        id.clone(),
        ClaimStatement::new(statement).expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(statement, None)),
    );

    store
        .create_asserted_claim(input)
        .expect("asserted claim creation should succeed");
}

//
// Verify trust inputs can represent all required reliability input categories.
#[test]
fn trust_input_kind_supports_required_reliability_categories() {
    let mut store = ClaimStore::new();

    let subject_source = "source://report/42";
    let subject_extractor = "extractor://pipeline/v1";
    let subject_model = "model://resolver/deterministic-v1";
    let subject_agent = "agent://analyst/alice";
    let subject_rule = "validation-rule://cyber/ioc-consistency";
    let subject_correction = "correction://history/retro-7";

    for subject in [
        subject_source,
        subject_extractor,
        subject_model,
        subject_agent,
        subject_rule,
        subject_correction,
    ] {
        store.register_trust_subject(subject.to_owned());
    }

    let cases = [
        (TrustInputKind::SourceReliability, subject_source),
        (TrustInputKind::ExtractorReliability, subject_extractor),
        (TrustInputKind::ModelReliability, subject_model),
        (TrustInputKind::AgentReliability, subject_agent),
        (TrustInputKind::ValidationRuleReliability, subject_rule),
        (TrustInputKind::HistoricalCorrection, subject_correction),
    ];

    for (kind, subject_ref) in cases {
        store
            .create_trust_input(TrustInputInput::new(kind, subject_ref.to_owned(), 0.72))
            .expect("trust input creation should succeed for known subject");
    }

    let rule_inputs = store
        .trust_inputs_by_subject(subject_rule)
        .expect("trust inputs should be readable by subject");
    assert_eq!(rule_inputs.len(), 1);
    assert_eq!(
        rule_inputs[0].kind(),
        TrustInputKind::ValidationRuleReliability
    );
}

//
// Verify trust inputs preserve provenance and temporal metadata.
#[test]
fn trust_inputs_capture_provenance_and_temporal_metadata() {
    let mut store = ClaimStore::new();
    let subject = "source://investigation/evidence-pack-1";
    store.register_trust_subject(subject.to_owned());

    let temporal = TemporalMetadata {
        created_at: Some("2026-07-06T08:40:00Z".to_owned()),
        observed_at: Some("2026-07-05T18:00:00Z".to_owned()),
        ..Default::default()
    };

    let trust_id = store
        .create_trust_input(
            TrustInputInput::new(TrustInputKind::SourceReliability, subject.to_owned(), 0.81)
                .with_provenance_ref("provenance://pipeline/step-9".to_owned())
                .with_reason_ref("reason://calibration/source-consistency".to_owned())
                .with_temporal(temporal.clone()),
        )
        .expect("trust input creation with provenance and temporal metadata should succeed");

    let subject_inputs = store
        .trust_inputs_by_subject(subject)
        .expect("subject lookup should succeed");

    assert_eq!(subject_inputs.len(), 1);
    assert_eq!(subject_inputs[0].trust_input_id(), trust_id);
    assert_eq!(
        subject_inputs[0].provenance_ref(),
        Some("provenance://pipeline/step-9")
    );
    assert_eq!(
        subject_inputs[0].reason_ref(),
        Some("reason://calibration/source-consistency")
    );
    assert_eq!(subject_inputs[0].temporal(), &temporal);
}

//
// Verify invalid trust input values are rejected with explicit typed errors.
#[test]
fn invalid_trust_input_values_return_typed_errors() {
    let mut store = ClaimStore::new();
    let subject = "agent://analyst/bob";
    store.register_trust_subject(subject.to_owned());

    let invalid = store
        .create_trust_input(TrustInputInput::new(
            TrustInputKind::AgentReliability,
            subject.to_owned(),
            1.4,
        ))
        .expect_err("out-of-range trust value should fail");

    assert!(matches!(
    invalid,
    GraphError::InvalidTrustInputValue(value) if value == 1.4
    ));
}

//
// Verify unknown trust subjects are rejected with explicit typed errors.
#[test]
fn unknown_trust_subject_returns_typed_error() {
    let mut store = ClaimStore::new();
    let unknown_subject = "model://unknown/123";

    let err = store
        .create_trust_input(TrustInputInput::new(
            TrustInputKind::ModelReliability,
            unknown_subject.to_owned(),
            0.41,
        ))
        .expect_err("unknown subject should fail");

    assert!(matches!(
    err,
    GraphError::TrustSubjectNotFound(subject) if subject == unknown_subject
    ));
}

//
// Verify claim-relevant trust inputs can be listed and do not automatically
// mutate claim lifecycle status.
#[test]
fn claim_trust_inputs_are_traceable_without_auto_resolving_claim_status() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--trust-does-not-auto-resolve");
    create_asserted_claim(
        &mut store,
        &claim,
        "Trust inputs should not auto-resolve this claim",
    );

    let subject = "source://report/auto-resolution-check";
    store.register_trust_subject(subject.to_owned());

    store
        .create_trust_input(
            TrustInputInput::new(TrustInputKind::SourceReliability, subject.to_owned(), 0.56)
                .with_claim_ref(claim.clone()),
        )
        .expect("trust input creation should succeed");

    let trust_for_claim = store
        .trust_inputs_for_claim(&claim)
        .expect("claim trust lookup should succeed");
    assert_eq!(trust_for_claim.len(), 1);

    let claim_after = store
        .claim_by_id(&claim)
        .expect("claim must remain readable after trust input attach");
    assert_eq!(claim_after.status(), ClaimStatus::Asserted);
}
