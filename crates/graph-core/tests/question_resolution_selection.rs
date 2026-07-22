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
    GraphError, QuestionIntent, ResolutionLevel, ResolutionSelectionReason,
    ResolutionSelectionRequest, select_question_resolution,
};

#[test]
fn typed_question_intents_map_to_expected_resolution_levels() {
    for (intent, expected_level) in [
        (
            QuestionIntent::TacticalEvidenceDetail,
            ResolutionLevel::Tactical,
        ),
        (
            QuestionIntent::OperationalCampaignAnalysis,
            ResolutionLevel::Operational,
        ),
        (
            QuestionIntent::StrategicObjectiveAssessment,
            ResolutionLevel::Strategic,
        ),
    ] {
        let request = ResolutionSelectionRequest::new("question--typed-mapping", vec![intent])
            .expect("selection request should be valid");
        let selection =
            select_question_resolution(&request).expect("typed intent should select a level");

        assert_eq!(selection.selected_level(), expected_level);
        assert_eq!(
            selection.trace().reason(),
            ResolutionSelectionReason::Direct
        );
        assert_eq!(selection.trace().intent_mappings().len(), 1);
    }
}

#[test]
fn ambiguous_intents_use_deterministic_most_detailed_tie_breaking() {
    let first = ResolutionSelectionRequest::new(
        "question--ambiguous",
        vec![
            QuestionIntent::StrategicObjectiveAssessment,
            QuestionIntent::TacticalEvidenceDetail,
            QuestionIntent::OperationalCampaignAnalysis,
        ],
    )
    .expect("selection request should be valid");
    let second = ResolutionSelectionRequest::new(
        "question--ambiguous",
        vec![
            QuestionIntent::OperationalCampaignAnalysis,
            QuestionIntent::TacticalEvidenceDetail,
            QuestionIntent::StrategicObjectiveAssessment,
        ],
    )
    .expect("selection request should be valid");

    let first_selection =
        select_question_resolution(&first).expect("ambiguous intents should be resolved");
    let second_selection =
        select_question_resolution(&second).expect("ambiguous intents should be resolved");

    assert_eq!(first_selection.selected_level(), ResolutionLevel::Tactical);
    assert_eq!(
        first_selection.trace().reason(),
        ResolutionSelectionReason::AmbiguousMostDetailed
    );
    assert_eq!(first_selection, second_selection);
    assert_eq!(
        serde_json::to_string(&first_selection).expect("selection should serialize"),
        serde_json::to_string(&second_selection).expect("selection should serialize")
    );
}

#[test]
fn matching_explicit_resolution_is_recorded_in_auditable_trace() {
    let request = ResolutionSelectionRequest::new(
        "question--explicit-operational",
        vec![QuestionIntent::OperationalCampaignAnalysis],
    )
    .expect("selection request should be valid")
    .with_requested_level(ResolutionLevel::Operational);

    let selection =
        select_question_resolution(&request).expect("matching explicit level should succeed");

    assert_eq!(selection.selected_level(), ResolutionLevel::Operational);
    assert_eq!(
        selection.trace().reason(),
        ResolutionSelectionReason::ExplicitMatch
    );
    assert_eq!(
        selection.trace().requested_level(),
        Some(ResolutionLevel::Operational)
    );
    assert_eq!(
        selection.trace().question_ref(),
        "question--explicit-operational"
    );
}

#[test]
fn conflicting_resolution_request_is_rejected_without_fallback() {
    let request = ResolutionSelectionRequest::new(
        "question--conflict",
        vec![QuestionIntent::TacticalEvidenceDetail],
    )
    .expect("selection request should be valid")
    .with_requested_level(ResolutionLevel::Strategic);

    let error = select_question_resolution(&request)
        .expect_err("conflicting explicit level must not silently override intent");

    assert!(matches!(error, GraphError::InvalidResolutionSelection(_)));
}

#[test]
fn unsupported_or_missing_intent_classification_is_rejected_without_fallback() {
    let unsupported = ResolutionSelectionRequest::new(
        "question--unsupported",
        vec![QuestionIntent::Unsupported("legal-review".to_owned())],
    )
    .expect("unsupported intent remains representable for typed validation");
    let unsupported_error = select_question_resolution(&unsupported)
        .expect_err("unsupported intent must not fall back to a default level");
    assert!(matches!(
        unsupported_error,
        GraphError::InvalidResolutionSelection(_)
    ));

    let missing_error = ResolutionSelectionRequest::new("question--missing", Vec::new())
        .expect_err("missing intent classification should fail at the request boundary");
    assert!(matches!(
        missing_error,
        GraphError::InvalidResolutionSelection(_)
    ));
}
