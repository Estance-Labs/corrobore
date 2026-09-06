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
use graph_core::{Confidence, ConfidenceDimension, ConfidenceDimensions, Verdict, VerdictStore};
use serde_json::json;

fn legacy(dimensions: serde_json::Value) -> serde_json::Value {
    let mut store = VerdictStore::new();
    let claim = graph_core::ClaimId::new("claim--dimensions").expect("valid fixture");
    let time = graph_core::TemporalTimestamp::new("2026-09-01T00:00:00Z").expect("valid fixture");
    store
        .append_verdict(
            graph_core::VerdictId::new("verdict--dimensions").expect("valid fixture"),
            claim.clone(),
            graph_core::VerdictState::Supported,
            ConfidenceDimensions::default(),
            "ws-a-minimal-v1",
            graph_core::BitemporalStamp::new(time.clone(), time).expect("valid fixture"),
        )
        .expect("valid fixture");
    let mut stored = serde_json::to_value(store.current_verdict(&claim).expect("valid fixture"))
        .expect("valid fixture");
    stored["confidence_dimensions"] = dimensions;
    stored
}

#[test]
fn absent_is_not_zero_and_empty_verdict_omits_dimensions() {
    let empty: Verdict = serde_json::from_value(legacy(json!({}))).expect("valid fixture");
    assert!(empty.confidence_dimensions().is_empty());
    let value = serde_json::to_value(&empty).expect("valid fixture");
    assert!(value.get("confidence_dimensions").is_none());
    assert_eq!(
        serde_json::from_value::<Verdict>(value).expect("valid fixture"),
        empty
    );
    let zero: Verdict =
        serde_json::from_value(legacy(json!({"evidence_sufficiency":0.0}))).expect("valid fixture");
    assert_eq!(
        zero.confidence_dimensions()
            .evidence_sufficiency
            .expect("valid fixture")
            .value(),
        0.0
    );
    assert!(zero.confidence_dimensions().source_authority.is_none());
    assert_ne!(empty.confidence_dimensions(), zero.confidence_dimensions());
}

#[test]
fn all_ten_dimensions_round_trip_and_project_only_present_values() {
    let values = json!({"evidence_sufficiency":0.0,"source_authority":0.1,"source_independence":0.2,
        "extraction_certainty":0.3,"entity_resolution_certainty":0.4,"temporal_validity":0.5,
        "contradiction_load":0.6,"verifier_strength":0.7,"epistemic_uncertainty":0.8,"actionability":1.0});
    let verdict: Verdict = serde_json::from_value(legacy(values.clone())).expect("valid fixture");
    assert_eq!(
        serde_json::to_value(verdict.confidence_dimensions()).expect("valid fixture"),
        values
    );
    assert_eq!(
        serde_json::from_value::<ConfidenceDimensions>(values.clone()).expect("valid fixture"),
        *verdict.confidence_dimensions()
    );
    for (key, value) in values.as_object().expect("valid fixture") {
        assert_eq!(
            verdict
                .to_property_map()
                .get(&format!("verdict_dimension_{key}")),
            Some(&graph_core::PropertyValue::Float(
                value.as_f64().expect("valid fixture")
            ))
        );
    }
    let sparse: Verdict =
        serde_json::from_value(legacy(json!({"actionability":0.0}))).expect("valid fixture");
    assert_eq!(
        sparse
            .to_property_map()
            .keys()
            .filter(|key| key.starts_with("verdict_dimension_"))
            .count(),
        1
    );
}

#[test]
fn favorable_values_invert_only_the_two_adverse_dimensions_and_preserve_absence() {
    let dimensions = ConfidenceDimensions {
        contradiction_load: Some(Confidence::new(0.75).expect("valid fixture")),
        epistemic_uncertainty: Some(Confidence::new(1.0).expect("valid fixture")),
        source_authority: Some(Confidence::new(0.75).expect("valid fixture")),
        ..Default::default()
    };
    assert_eq!(
        dimensions
            .favorable_value(ConfidenceDimension::ContradictionLoad)
            .expect("valid fixture")
            .value(),
        0.25
    );
    assert_eq!(
        dimensions
            .favorable_value(ConfidenceDimension::EpistemicUncertainty)
            .expect("valid fixture")
            .value(),
        0.0
    );
    assert_eq!(
        dimensions
            .favorable_value(ConfidenceDimension::SourceAuthority)
            .expect("valid fixture")
            .value(),
        0.75
    );
    assert_eq!(
        dimensions.favorable_value(ConfidenceDimension::Actionability),
        None
    );
}

#[test]
fn legacy_unknown_keys_produce_persistent_findings_once_per_key() {
    let store = json!({"verdicts":[legacy(json!({"evidence_sufficiency":0.5,"custom_a":0.2,"custom_b":0.0}))], "transitions":[]});
    let migrated: VerdictStore = serde_json::from_value(store).expect("valid fixture");
    let claim = graph_core::ClaimId::new("claim--dimensions").expect("valid fixture");
    let verdict = migrated.current_verdict(&claim).expect("valid fixture");
    assert_eq!(verdict.dimension_migration_findings().len(), 2);
    for (finding, key) in verdict
        .dimension_migration_findings()
        .iter()
        .zip(["custom_a", "custom_b"])
    {
        assert_eq!(finding.verdict_id(), verdict.id());
        assert_eq!(finding.key(), key);
        let record = finding.to_validation_record();
        assert!(record.message().contains(key));
        assert!(record.message().contains(verdict.id().as_str()));
    }
    let serialized = serde_json::to_value(&migrated).expect("valid fixture");
    assert_eq!(
        serialized["verdicts"][0]["confidence_dimensions"],
        json!({"evidence_sufficiency":0.5})
    );
    let reloaded: VerdictStore = serde_json::from_value(serialized.clone()).expect("valid fixture");
    assert_eq!(migrated, reloaded);
    assert_eq!(
        serialized,
        serde_json::to_value(reloaded).expect("valid fixture")
    );
}

#[test]
fn standalone_record_rejects_unknown_dimensions_and_invalid_scores() {
    assert!(serde_json::from_value::<ConfidenceDimensions>(json!({"typo":0.5})).is_err());
    for value in [-0.1, 1.1] {
        assert!(serde_json::from_value::<Verdict>(legacy(json!({"actionability":value}))).is_err());
    }
}
