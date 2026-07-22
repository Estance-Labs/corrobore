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
use graph_core::{GraphError, SemanticSeedResolutionError, SemanticSeedResolutionErrorCode};

//
// Verify semantic seed no-result failures are explicit, typed, and matchable.
#[test]
fn semantic_seed_no_seed_error_is_stable_and_matchable() {
    let error = GraphError::SemanticSeedResolutionFailed(SemanticSeedResolutionError::new(
        SemanticSeedResolutionErrorCode::NoSeed,
        "No seed candidate matched the semantic objective.",
        "Narrow objective scope or lower score threshold.",
    ));

    assert!(matches!(
        error,
        GraphError::SemanticSeedResolutionFailed(SemanticSeedResolutionError {
            code: SemanticSeedResolutionErrorCode::NoSeed,
            ..
        })
    ));
}

//
// Verify ambiguous-seed and overbroad-objective failures preserve structured
// details that callers can inspect deterministically.
#[test]
fn semantic_seed_error_payload_supports_deterministic_diagnostic_metadata() {
    let ambiguous = SemanticSeedResolutionError::new(
        SemanticSeedResolutionErrorCode::AmbiguousSeed,
        "Multiple seed candidates have near-identical ranking.",
        "Add domain profile constraints or provide a stronger objective.",
    )
    .with_candidate_count(12)
    .with_threshold(0.65);

    assert_eq!(
        ambiguous.code,
        SemanticSeedResolutionErrorCode::AmbiguousSeed
    );
    assert_eq!(ambiguous.candidate_count, Some(12));
    assert_eq!(ambiguous.threshold, Some(0.65));

    let overbroad = SemanticSeedResolutionError::new(
        SemanticSeedResolutionErrorCode::OverbroadObjective,
        "Objective matches too many high-degree entities.",
        "Add stronger qualifiers before building a working set.",
    )
    .with_candidate_count(1_250)
    .with_threshold(0.20);

    assert_eq!(
        overbroad.code,
        SemanticSeedResolutionErrorCode::OverbroadObjective
    );
    assert_eq!(overbroad.candidate_count, Some(1_250));
    assert_eq!(overbroad.threshold, Some(0.20));
}
